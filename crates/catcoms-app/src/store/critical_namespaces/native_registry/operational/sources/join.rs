//! Affine capture for the first accepted request on a mounted native vault.
use super::super::super::lifecycle::joined::LocalNames;
use super::super::governed::{
    connect_approved_host, receive_connected_join, ApprovedContact, ConnectedJoinDelivery,
    JoinFramePayment, NativeTextJoinedStorage,
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
use crate::store::persistence::v2_member_outbox::{MemberHistoryRead, MemberOutboxContextId};
use crate::store::persistence::{
    prepare_joined_member_history, JoinedMemberLane, JoinedMemberSource,
};
use crate::store::{
    MemberSaveJob, MemberSaveJobReport, MemberSaveNativeAdmission, NativeTextInvitePacketData,
    NativeTextJoinedRegistrationData, NativeTextJoinedRegistryRowData, NativeTextMemberHistoryPage,
    V2MemberSaveOutcome, V2MemberSaveRefusal, V2MemberSignRequest,
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
    sync::atomic::{AtomicBool, Ordering},
    sync::{OnceLock, Weak},
    time::Duration,
};
use tokio::sync::watch;

mod publication;
pub use publication::{
    NativeTextJoinedMemberPublication, NativeTextJoinedMemberPublicationReceipt,
};
mod receive;
pub use receive::{
    NativeTextJoinedArchivedActions, NativeTextJoinedDependencyExchange,
    NativeTextJoinedDependencyProvider, NativeTextJoinedDependencyResponse,
    NativeTextJoinedGoverningExchange, NativeTextJoinedGoverningResponse,
    NativeTextJoinedMemberIngressPreparation, NativeTextJoinedMemberLedger,
    NativeTextJoinedMemberProjection, NativeTextJoinedMemberReceiver,
    NativeTextJoinedReceivedOriginal,
};
mod reconnect;
pub use reconnect::NativeTextJoinedReconnect;

#[derive(Default)]
pub(super) struct PendingJoin {
    state: Mutex<PendingJoinState>,
    close: Arc<OnceLock<NativeChatSourceClose>>,
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

    fn ensure_native_owner(
        &self,
        state: &mut PendingJoinState,
        command: &NativeTextSourceCommand,
        control: &Arc<retirement::Control>,
    ) -> Result<(), AppError> {
        if state.owner.is_none() {
            let owner =
                NativeChatSourceOwner::for_trusted_source(&command.ui.command, control.clone())
                    .map_err(|_| refused())?;
            self.close
                .set(owner.close_handle())
                .map_err(|_| refused())?;
            state.owner = Some(owner);
            if !command.scope.is_open() {
                self.close_keys();
            }
        }
        Ok(())
    }
}
pub(super) type MountedJoin<R> = Mutex<Option<MountedNode<R>>>;

pub(super) fn invoice<R: CryptoRngCore>() -> usize {
    2 * std::mem::size_of::<NativeTextJoinedRequestPreparation>()
        + 2 * std::mem::size_of::<NativeTextJoinedDelivery>()
        + std::mem::size_of::<PendingJoin>()
        + std::mem::size_of::<OnceLock<NativeChatSourceClose>>() + 2 * std::mem::size_of::<usize>()
        + 2 * std::mem::size_of::<NativeTextJoinedInstallation<R>>()
        + 2 * std::mem::size_of::<NativeTextJoinedSelectionPreparation<R>>()
        + std::mem::size_of::<MountedJoin<R>>()
        + std::mem::size_of::<join_tcp::Transport>()
        + 2 * std::mem::size_of::<NativeChatKeyEnds>()
        + std::mem::size_of::<JoinedRowSelection>() + 512 // bounded original row name and its handles
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
    /// The original command and source are still current, but the physical
    /// metadata slot was occupied at capture. This cannot admit any work.
    pub fn is_capture_busy_cached(&self) -> bool {
        self.end.is_some()
            && Arc::ptr_eq(&self.scope, &self.command.scope)
            && self.scope.is_open()
            && self.command.ui.is_current_cached()
            && self.work.is_none()
    }

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
        let recover = pending.io.is_none() && work.has_covered_join_request()?;
        if token == [0; 32]
            || anchor != &invite.binding().spec_data().anchor
            || (pending.io.is_none() && !recover && invite.binding().expired_at_data(now_ms))
        {
            return Err(refused());
        }
        if pending.owner.is_none() {
            require_current()?;
            original.ensure_native_owner(&mut pending, &self.command, &self._control)?;
            require_current()?;
        }
        let owner = pending.owner.as_ref().ok_or_else(refused)?;
        let source = owner.observe();
        let purpose = if recover {
            NativeChatKeyPurpose::JoinerPendingSource
        } else {
            NativeChatKeyPurpose::JoinerDevice
        };
        let mut cut = owner
            .prepare_cut(&self.command.ui.command, purpose)
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
            row: None,
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
    row: Option<NativeTextJoinedRowSelection>,
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
            && self
                .row
                .as_ref()
                .is_none_or(NativeTextJoinedRowSelection::is_current_cached)
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
            ApprovedContact::from_invite(invite, approved_host),
            &self.tcp.borrow()?.capture_connection_dial(),
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
    #[cfg(test)]
    pub(in crate::store::critical_namespaces::native_registry) fn wire_for_test(&self) -> &[u8] {
        &self.delivery.bytes
    }

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
    member: Option<JoinedMemberLane>,
    node: Option<ClosedV2JoinedNode<catcoms_net::MeshService, R>>,
    registration: Option<Arc<JoinedRegistrationDependency>>,
    recovered: Option<NativeTextJoinedRegistrationData>,
    selection: Option<NativeTextJoinedRowSelection>,
    founder_contact: OnceLock<PeerId>,
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
    pub(in super::super) fn install_recovered_joined(
        &mut self,
        mut node: Option<ClosedV2JoinedNode<catcoms_net::MeshService, R>>,
        receipt: NativeTextJoinedRegistrationData,
        transport: Arc<join_tcp::Transport>,
        payment: CatalogReadReservation,
    ) -> Result<(), AppError> {
        let checked = node
            .as_ref()
            .filter(|node| node.was_recovered_data())
            .ok_or_else(refused)?;
        if !self.scope.is_open()
            || !self.work.is_live_cached()
            || checked.selected_join_data().is_err()
            || checked.original_join_data() != receipt.selected_data()
            || checked.local_peer_data() != receipt.transport_data().0
        {
            return Err(refused());
        }
        let slot = Arc::get_mut(&mut self.mounted_join)
            .ok_or_else(refused)?
            .get_mut()
            .map_err(|_| refused())?;
        if slot.is_some() {
            return Err(refused());
        }
        self.joined_tcp = transport;
        *slot = Some(MountedNode {
            member: None,
            node: node.take(),
            registration: None,
            recovered: Some(receipt),
            selection: None,
            founder_contact: OnceLock::new(),
            _payment: payment,
        });
        Ok(())
    }

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
                    member: None,
                    node,
                    registration: None,
                    recovered: None,
                    selection: None,
                    founder_contact: OnceLock::from(self.exchange.target),
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

/// One actual joined append coupled to the original counted native Source work.
pub struct NativeTextJoinedMemberSaveJob<R: CryptoRngCore> {
    job: MemberSaveJob,
    _original: NativeTextJoinedMemberFinalPreparation<R>,
}

pub struct NativeTextJoinedMemberFinalPreparation<R: CryptoRngCore> {
    selection: NativeTextJoinedRowSelection,
    token: [u8; 32],
    original: NativeTextJoinedSelectionPreparation<R>,
}

impl<R: CryptoRngCore> NativeTextJoinedMemberSaveJob<R> {
    pub fn run(self) -> MemberSaveJobReport {
        self.job.run()
    }
}

/// Paid original row association issued by the covered mounted runtime.
/// It exposes data and an irreversible close fence while retaining no node.
#[derive(Clone)]
pub struct NativeTextJoinedRowSelection {
    original: Arc<JoinedRowSelection>,
}

struct JoinedRowSelection {
    row: NativeTextJoinedRegistryRowData,
    names: LocalNames,
    request: [u8; 32],
    member_outbox: MemberOutboxContextId,
    scope: Arc<Scope>,
    retired: AtomicBool,
    keys: Arc<OnceLock<NativeChatSourceClose>>,
    _metadata: Arc<retirement::Control>,
}

impl std::fmt::Debug for NativeTextJoinedRowSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeTextJoinedRowSelection")
            .finish_non_exhaustive()
    }
}

impl NativeTextJoinedRowSelection {
    pub fn id_data(&self) -> u64 {
        self.original.row.id
    }
    pub fn name_data(&self) -> &str {
        &self.original.names.group
    }
    pub fn display_name_data(&self) -> Option<&str> {
        self.original.names.display.as_deref()
    }
    pub fn request_data(&self) -> [u8; 32] {
        self.original.request
    }

    /// Stable outbox identity derived from the original joined pin and Profile.
    pub fn member_outbox_context_id_data(&self) -> [u8; 32] {
        *self.original.member_outbox.as_bytes()
    }
    pub fn same_original(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.original, &other.original)
    }
    pub fn is_current_cached(&self) -> bool {
        self.original.scope.is_open() && !self.original.retired.load(Ordering::SeqCst)
    }
    pub fn is_from_original_vault_cached(&self, vault: &NativeTextVault) -> bool {
        Arc::ptr_eq(&self.original.scope, &vault.sources.observe())
    }
    pub fn close_original_cached(&self) {
        self.original.retired.store(true, Ordering::SeqCst);
        if let Some(keys) = self.original.keys.get() {
            keys.close();
        }
    }
}

impl<R: CryptoRngCore> std::fmt::Debug for NativeTextJoinedSelectionPreparation<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeTextJoinedSelectionPreparation")
            .finish_non_exhaustive()
    }
}

impl<R: CryptoRngCore> NativeTextJoinedSelectionPreparation<R> {
    pub fn is_capture_busy_cached(&self) -> bool {
        self.original.is_capture_busy_cached()
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

    /// The trusted startup owner can prepare a single recovered row before
    /// installing its reauthenticated vault and Sources pair. This proves
    /// the registered closed graph and grants no command or signing authority.
    pub fn prepare_recovered_joined_row(
        &mut self,
        vault: &NativeTextVault,
        authentication: &crate::store::NativeTextVaultAuthenticationCheck,
    ) -> Result<NativeTextJoinedRowSelection, AppError> {
        if !self.scope.is_open()
            || !Arc::ptr_eq(&self.scope, &vault.sources.observe())
            || !authentication.is_for_original_vault(vault)
        {
            return Err(refused());
        }
        let metadata = self.work.metadata().ok_or_else(refused)?;
        let slot = Arc::get_mut(&mut self.mounted_join)
            .ok_or_else(refused)?
            .get_mut()
            .map_err(|_| refused())?;
        let runtime = slot.as_mut().ok_or_else(refused)?;
        let data = runtime.recovered.ok_or_else(refused)?;
        if !runtime
            .node
            .as_ref()
            .is_some_and(|node| node.was_recovered_data())
        {
            return Err(refused());
        }
        let (row, name) = runtime.observe_selected(&metadata, data)?;
        if !authentication.is_for_original_vault(vault) {
            return Err(refused());
        }
        runtime.select(
            row,
            name,
            data,
            &self.scope,
            self.pending_join.close.clone(),
            &self.retirement,
        )
    }
}

impl<R: CryptoRngCore> MountedNode<R> {
    fn registration_data(&self) -> Option<NativeTextJoinedRegistrationData> {
        self.recovered
            .or_else(|| self.registration.as_ref().map(|proof| proof.data()))
    }
    fn matches_selection(&self, selection: &NativeTextJoinedRowSelection) -> bool {
        self.selection
            .as_ref()
            .is_some_and(|original| original.same_original(selection))
    }

    fn observe_selected(
        &self,
        metadata: &MetadataWork,
        data: NativeTextJoinedRegistrationData,
    ) -> Result<(NativeTextJoinedRegistryRowData, LocalNames), AppError> {
        let actual = self.node.as_ref().ok_or_else(refused)?;
        if actual.selected_join_data().is_err()
            || data.selected_data() != actual.original_join_data()
        {
            return Err(refused());
        }
        metadata.observe_registered_joined_row(data, self.registration.as_deref())
    }

    fn select(
        &mut self,
        row: NativeTextJoinedRegistryRowData,
        names: LocalNames,
        data: NativeTextJoinedRegistrationData,
        scope: &Arc<Scope>,
        keys: Arc<OnceLock<NativeChatSourceClose>>,
        control: &Arc<retirement::Control>,
    ) -> Result<NativeTextJoinedRowSelection, AppError> {
        let member_outbox = data.member_outbox_context_data()?.id();
        let selection = self
            .selection
            .get_or_insert_with(|| NativeTextJoinedRowSelection {
                original: Arc::new(JoinedRowSelection {
                    row,
                    names,
                    request: data.request,
                    member_outbox,
                    scope: scope.clone(),
                    retired: AtomicBool::new(false),
                    keys,
                    _metadata: control.clone(),
                }),
            });
        if selection.original.row != row
            || selection.original.request != data.request
            || selection.original.member_outbox != member_outbox
            || !selection.is_current_cached()
        {
            return Err(refused());
        }
        Ok(selection.clone())
    }
}

impl<R: CryptoRngCore> NativeTextJoinedSelectionPreparation<R> {
    /// Prepare or recover the exact ordinary member action under its original
    /// covered joined files. Only the warm Source can enter the member key.
    pub fn prepare_member_save(
        self,
        selection: &NativeTextJoinedRowSelection,
        request: V2MemberSignRequest<'_>,
        negative: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<V2MemberSaveOutcome, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        let mut ends = NativeChatKeyEnds::new();
        self.original
            .require_current(negative)
            .map_err(|_| Refusal::Cancelled)?;
        if !selection.is_current_cached() {
            return Err(Refusal::Stale);
        }
        let original = self
            .original
            .pending
            .upgrade()
            .ok_or(Refusal::Unavailable)?;
        let mut pending = original.state.try_lock().map_err(|_| Refusal::Busy)?;
        let mounted = self.mounted.upgrade().ok_or(Refusal::Unavailable)?;
        let mut slot = mounted.try_lock().map_err(|_| Refusal::Busy)?;
        let runtime = slot
            .as_mut()
            .filter(|runtime| runtime.matches_selection(selection))
            .ok_or(Refusal::Stale)?;
        let metadata = self.original.work.as_ref().ok_or(Refusal::Unavailable)?;
        let recovered = runtime.recovered;
        if let Some(receipt) = recovered.filter(|_| runtime.member.is_none()) {
            let saved = metadata
                .lookup_saved_joined_member(receipt, None, selection.original.row, &request, rng)
                .map_err(|_| Refusal::Failed)?;
            self.original
                .require_current(negative)
                .map_err(|_| Refusal::Cancelled)?;
            if let Some((action, ordinal)) = saved {
                return Ok(V2MemberSaveOutcome::Saved { action, ordinal });
            }
        }
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
        let held = pending.io.as_ref();
        let owner = pending.owner.as_ref().ok_or(Refusal::Unavailable)?;
        let native = owner.observe();
        let mut cut = owner
            .prepare_cut(
                &self.original.command.ui.command,
                NativeChatKeyPurpose::MemberAction,
            )
            .map_err(|_| Refusal::Unavailable)?;
        let data = runtime.registration_data().ok_or(Refusal::Unavailable)?;
        let MountedNode {
            member,
            node,
            registration,
            ..
        } = runtime;
        let proof = registration.as_deref();
        let node = node.as_mut().ok_or(Refusal::Unavailable)?;
        let result = metadata.with_registered_joined_member(
            data,
            proof,
            selection.original.row,
            |layout, store| {
                let recovered_prefix = recovered
                    .map(|data| store.capture_recovered_member_prefix(data))
                    .transpose()
                    .map_err(|_| Refusal::Unavailable)?;
                let (prefix, view) = match &recovered_prefix {
                    Some(original) => {
                        let (prefix, view) = original.member_parts();
                        node.bind_recovered_member_source(prefix, &native, &cut)
                            .map_err(|_| Refusal::Unavailable)?;
                        (prefix, view)
                    }
                    None => held
                        .ok_or(Refusal::Unavailable)?
                        .borrow_warm_member_prefix(store)
                        .map_err(|_| Refusal::Unavailable)?,
                };
                if member.is_none() {
                    *member = Some(
                        JoinedMemberLane::new(store, selection.clone())
                            .map_err(|_| Refusal::Busy)?,
                    );
                }
                self.original
                    .require_current(negative)
                    .map_err(|_| Refusal::Cancelled)?;
                let source = JoinedMemberSource {
                    node,
                    prefix,
                    view,
                    native: &native,
                    cut: &mut cut,
                    ends: &mut ends,
                };
                member
                    .as_mut()
                    .ok_or(Refusal::Unavailable)?
                    .prepare_member_save(layout, request, source, data, negative, rng)
            },
        )?;
        self.original
            .require_current(negative)
            .map_err(|_| Refusal::Cancelled)?;
        if !selection.is_current_cached() {
            return Err(Refusal::Stale);
        }
        Ok(result)
    }

    /// Reprove the exact registered row and its held physical files outside
    /// the native commit guard, using the second original capture.
    pub fn finish_member_admission(
        self,
        selection: NativeTextJoinedRowSelection,
        token: [u8; 32],
        negative: &RequestCancellation,
    ) -> Result<NativeTextJoinedMemberFinalPreparation<R>, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        self.original
            .require_current(negative)
            .map_err(|_| Refusal::Cancelled)?;
        {
            if !selection.is_current_cached() {
                return Err(Refusal::Stale);
            }
            let mounted = self.mounted.upgrade().ok_or(Refusal::Unavailable)?;
            let slot = mounted.try_lock().map_err(|_| Refusal::Busy)?;
            let runtime = slot
                .as_ref()
                .filter(|runtime| runtime.matches_selection(&selection))
                .ok_or(Refusal::Stale)?;
            let data = runtime
                .registration_data()
                .filter(|_| runtime.member.is_some())
                .ok_or(Refusal::Stale)?;
            let metadata = self.original.work.as_ref().ok_or(Refusal::Unavailable)?;
            if runtime
                .observe_selected(metadata, data)
                .map_err(|_| Refusal::Unavailable)?
                .0
                != selection.original.row
            {
                return Err(Refusal::Stale);
            }
        }
        self.original
            .require_current(negative)
            .map_err(|_| Refusal::Cancelled)?;
        Ok(NativeTextJoinedMemberFinalPreparation {
            selection,
            token,
            original: self,
        })
    }
}

impl<R: CryptoRngCore> NativeTextJoinedMemberFinalPreparation<R> {
    /// Cached final edge. A refusal leaves the original preparation in its
    /// outer slot so native commit guards release before any Source work drops.
    pub fn try_admit_member_save(
        original: &mut Option<Self>,
        negative: &RequestCancellation,
        native: MemberSaveNativeAdmission,
    ) -> Result<NativeTextJoinedMemberSaveJob<R>, (MemberSaveNativeAdmission, V2MemberSaveRefusal)>
    {
        use V2MemberSaveRefusal as Refusal;
        let Some(final_cut) = original.as_ref() else {
            return Err((native, Refusal::Unavailable));
        };
        let preparation = &final_cut.original;
        let selection = &final_cut.selection;
        if preparation.original.require_current(negative).is_err() || !selection.is_current_cached()
        {
            return Err((native, Refusal::Cancelled));
        }
        let Some(mounted) = preparation.mounted.upgrade() else {
            return Err((native, Refusal::Unavailable));
        };
        let Ok(slot) = mounted.try_lock() else {
            return Err((native, Refusal::Busy));
        };
        let Some(runtime) = slot
            .as_ref()
            .filter(|runtime| runtime.matches_selection(selection))
        else {
            return Err((native, Refusal::Stale));
        };
        let (Some(member), Some(node), Some(metadata)) = (
            runtime.member.as_ref(),
            runtime.node.as_ref(),
            preparation.original.work.as_ref(),
        ) else {
            return Err((native, Refusal::Stale));
        };
        let job = metadata.with_joined_member_final(native, |layout, native| {
            member.admit_member_save(layout, selection, node, final_cut.token, native)
        })?;
        Ok(NativeTextJoinedMemberSaveJob {
            job,
            _original: original.take().expect("checked original joined work"),
        })
    }
}

impl<R: CryptoRngCore> NativeTextJoinedSelectionPreparation<R> {
    /// Read a bounded page of the selected joined member's saved originals.
    /// Warm and cold rows use the same authenticated outbox context.
    pub fn prepare_member_history(
        self,
        selection: &NativeTextJoinedRowSelection,
        start: u64,
        end: Option<u64>,
        maximum: usize,
        negative: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<NativeTextMemberHistoryPage, V2MemberSaveRefusal> {
        self.prepare_history(
            selection,
            MemberHistoryRead::Originals {
                start,
                end,
                maximum,
            },
            negative,
            rng,
        )
    }

    pub fn prepare_own_message_revision(
        self,
        selection: &NativeTextJoinedRowSelection,
        channel: u128,
        target: catcoms_replication::chat::ActionId,
        negative: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<NativeTextMemberHistoryPage, V2MemberSaveRefusal> {
        self.prepare_history(
            selection,
            MemberHistoryRead::OwnRevision { channel, target },
            negative,
            rng,
        )
    }

    fn prepare_history(
        self,
        selection: &NativeTextJoinedRowSelection,
        request: MemberHistoryRead,
        negative: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<NativeTextMemberHistoryPage, V2MemberSaveRefusal> {
        use V2MemberSaveRefusal as Refusal;
        self.original
            .require_current(negative)
            .map_err(|_| Refusal::Cancelled)?;
        if !selection.is_current_cached() {
            return Err(Refusal::Stale);
        }
        let mounted = self.mounted.upgrade().ok_or(Refusal::Unavailable)?;
        let slot = mounted.try_lock().map_err(|_| Refusal::Busy)?;
        let runtime = slot
            .as_ref()
            .filter(|runtime| runtime.matches_selection(selection))
            .ok_or(Refusal::Stale)?;
        let data = runtime
            .recovered
            .or_else(|| runtime.registration.as_ref().map(|proof| proof.data()))
            .ok_or(Refusal::Unavailable)?;
        let metadata = self.original.work.as_ref().ok_or(Refusal::Unavailable)?;
        let page = metadata.with_registered_joined_member(
            data,
            runtime.registration.as_deref(),
            selection.original.row,
            |layout, _| prepare_joined_member_history(layout, request, negative, rng, data),
        )?;
        self.original
            .require_current(negative)
            .map_err(|_| Refusal::Cancelled)?;
        if !selection.is_current_cached() {
            return Err(Refusal::Stale);
        }
        Ok(NativeTextMemberHistoryPage::from_original(page))
    }

    /// Observe the one original parked slot before choosing a retry phase.
    /// Actual contact and registry writes still verify their exact inputs.
    pub fn has_parked_runtime(self, negative: &RequestCancellation) -> Result<bool, AppError> {
        self.original.require_current(negative)?;
        let mounted = self.mounted.upgrade().ok_or_else(refused)?;
        let slot = mounted.try_lock().map_err(|_| refused())?;
        let parked = slot.as_ref().is_some_and(|runtime| runtime.node.is_some());
        self.original.require_current(negative)?;
        Ok(parked)
    }

    /// Look up the exact local request in the selected joined row's outbox.
    /// This can recover existing bytes but never opens signing or appends.
    pub fn lookup_saved_member(
        self,
        selection: &NativeTextJoinedRowSelection,
        request: &crate::store::V2MemberSignRequest<'_>,
        negative: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Option<(catcoms_replication::chat::ActionId, u64)>, AppError> {
        let require_current = || {
            self.original.require_current(negative)?;
            if !selection.is_current_cached() {
                return Err(refused());
            }
            Ok(())
        };
        require_current()?;
        let mounted = self.mounted.upgrade().ok_or_else(refused)?;
        let slot = mounted.try_lock().map_err(|_| refused())?;
        let runtime = slot
            .as_ref()
            .filter(|runtime| runtime.matches_selection(selection))
            .ok_or_else(refused)?;
        let data = runtime
            .recovered
            .or_else(|| runtime.registration.as_ref().map(|proof| proof.data()))
            .ok_or_else(refused)?;
        let metadata = self.original.work.as_ref().ok_or_else(refused)?;
        let result = metadata.lookup_saved_joined_member(
            data,
            runtime.registration.as_deref(),
            selection.original.row,
            request,
            rng,
        )?;
        require_current()?;
        Ok(result)
    }

    /// Borrow the same paid association only after proving the exact joined
    /// row against its original parked, selected node and current UI.
    pub fn select_registered(
        self,
        negative: &RequestCancellation,
    ) -> Result<NativeTextJoinedRowSelection, AppError> {
        self.original.require_current(negative)?;
        let pending = self.original.pending.upgrade().ok_or_else(refused)?;
        let _owner = pending.state.try_lock().map_err(|_| refused())?;
        let mounted = self.mounted.upgrade().ok_or_else(refused)?;
        let mut slot = mounted.try_lock().map_err(|_| refused())?;
        let runtime = slot.as_mut().ok_or_else(refused)?;
        let data = runtime
            .recovered
            .or_else(|| runtime.registration.as_ref().map(|proof| proof.data()))
            .ok_or_else(refused)?;
        let metadata = self.original.work.as_ref().ok_or_else(refused)?;
        let (row, name) = runtime.observe_selected(metadata, data)?;
        self.original.require_current(negative)?;
        runtime.select(
            row,
            name,
            data,
            &self.original.scope,
            pending.close.clone(),
            &self.original._control,
        )
    }

    /// Select the native metadata row for the exact parked original. This
    /// commits local registration while joined authoring remains closed.
    pub fn register(
        self,
        invite: &NativeTextInvitePacketData<'_>,
        token: [u8; 32],
        name: String,
        display: String,
        negative: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<NativeTextJoinedRegistryRowData, AppError> {
        self.original.require_current(negative)?;
        let (receipt, debt) = {
            let original = self.original.pending.upgrade().ok_or_else(refused)?;
            let pending = original.state.try_lock().map_err(|_| refused())?;
            let mounted = self.mounted.upgrade().ok_or_else(refused)?;
            let mut slot = mounted.try_lock().map_err(|_| refused())?;
            let runtime = slot.as_mut().ok_or_else(refused)?;
            let node = runtime.node.as_ref().ok_or_else(refused)?;
            node.selected_join_data().map_err(|_| refused())?;
            let selected = node.original_join_data();
            let metadata = self.original.work.as_ref().ok_or_else(refused)?;
            if let Some(recovered) = runtime.recovered {
                if recovered.invocation != token
                    || recovered.invite != invite.binding().id_data()
                    || recovered.selected_data() != selected
                {
                    return Err(refused());
                }
                (
                    metadata.observe_registered_joined_row(recovered, None)?.0,
                    None,
                )
            } else {
                let pending = pending.io.as_ref().ok_or_else(refused)?;
                pending.require_original_join_intent(invite.binding(), token)?;
                if runtime.registration.is_none() {
                    runtime.registration = Some(metadata.capture_join_registration(pending)?);
                }
                let proof = runtime.registration.as_ref().ok_or_else(refused)?;
                if proof.data().selected_data() != selected || proof.data().invocation != token {
                    return Err(refused());
                }
                self.original.require_current(negative)?;
                metadata.register_joined(name, display, proof, rng)?
            }
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
            node.selected_join_data().map_err(|_| refused())?;
            let metadata = self.original.work.as_ref().ok_or_else(refused)?;
            let receipt = if let Some(recovered) = runtime.recovered {
                if recovered.invocation != token || recovered.invite != invite.binding().id_data() {
                    return Err(refused());
                }
                metadata.require_recovered_joined(recovered)?;
                recovered
            } else {
                metadata.observe_join_installation(
                    pending.io.as_ref().ok_or_else(refused)?,
                    invite,
                    token,
                )?
            };
            if receipt.selected_data() != node.original_join_data() {
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
