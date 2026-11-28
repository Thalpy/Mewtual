//! Explicit governed cold storage, with no signer/Source/native command or
//! facade conversion. Operational cold authoring needs its separate owner.
catcoms_rt::redacted_debug! {
    NativeTextJoinedStorage;
}

use super::super::lifecycle::preparation::Policy;
use super::*;
use crate::store::persistence::governance_layout::joined::{
    AcceptedFirstJoin, AcceptedJoinTransport, JoinedPendingIo, JoinedPinDeclaration,
};
use catcoms_replication::chat::join_bindings::{GenesisAnchorBindingData, SignedInviteBindingData};
use catcoms_rt::{
    ConnectionLease, CryptoRngCore, MeshTransport, ProtocolId, RequestAuthority,
    RequestCancellation, CHAT_V2_JOIN_KIND, CHAT_V2_JOIN_REQUEST_MAX_BYTES,
    CHAT_V2_JOIN_RESPONSE_MAX_BYTES,
};
use catcoms_sync::{
    PreparedV2JoinRequest, V2JoinRequestPacketData, V2PendingJoinContextData,
    MAX_V2_FIRST_JOIN_DELIVERY_BYTES, MAX_V2_JOIN_REQUEST_PACKET_BYTES,
};
mod contact;
mod founder_recovery;
mod runtime;
pub(super) use contact::{connect_approved_host, ApprovedContact};
pub use founder_recovery::{
    NativeTextColdFounderGoverningRequest, NativeTextColdFounderGoverningRequests,
    NativeTextColdFounderJoinRequest, NativeTextColdFounderJoinRequests,
    NativeTextColdFounderMemberIngress, NativeTextColdFounderMemberReceiver,
    NativeTextColdFounderPublication, NativeTextColdFounderPublicationReceipt,
    NativeTextColdFounderSource, NativeTextFounderReviewExchange, NativeTextFounderReviewPage,
    NativeTextFounderReviewReply, NativeTextFounderReviewResult, NativeTextFounderReviewSweep,
};
pub use runtime::{
    NativeTextJoinedRecovery, NativeTextJoinedRuntime, NativeTextRejoinDelivery,
    NativeTextRejoinExchange, NativeTextRemovedJoinedVault,
};
use std::sync::atomic::{AtomicBool, AtomicU64};

pub(in crate::store::critical_namespaces::native_registry) const JOIN_PROTOCOL: ProtocolId =
    ProtocolId("/catcoms/rr/1");
const _: () = assert!(MAX_V2_JOIN_REQUEST_PACKET_BYTES < CHAT_V2_JOIN_REQUEST_MAX_BYTES);
const _: () = assert!(MAX_V2_FIRST_JOIN_DELIVERY_BYTES <= CHAT_V2_JOIN_RESPONSE_MAX_BYTES);

#[derive(Debug)]
struct JoinConnectionAuthority {
    connection: ConnectionLease,
    mount: RequestCancellation,
    caller: RequestCancellation,
}

pub(super) struct JoinFramePayment {
    pub(super) _fee: CatalogReadReservation,
}

pub(super) struct ConnectedJoinDelivery {
    pub(super) bytes: bytes::Bytes,
    pub(super) scope: RequestCancellation,
}

impl AsRef<[u8]> for ConnectedJoinDelivery {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

impl std::fmt::Debug for JoinFramePayment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JoinFramePayment").finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl RequestAuthority for JoinConnectionAuthority {
    fn authorized(&self) -> bool {
        self.connection.is_live() && !self.mount.is_cancelled() && !self.caller.is_cancelled()
    }

    async fn revoked(&self) {
        let mut mount = self.mount.clone();
        let mut caller = self.caller.clone();
        tokio::select! {
            _ = self.connection.revoked() => {},
            _ = mount.cancelled() => {},
            _ = caller.cancelled() => {},
        }
    }
}

/// Original opaque combined4+5+native3 physical custody. Metadata reads do not
/// reconstruct a signer, reuse fresh Welcome evidence or grant a native caller.
pub struct NativeTextGovernedStorage {
    joined_declaration: Option<JoinedPinDeclaration>,
    index: upgrade_source::owning::Store,
}
impl std::fmt::Debug for NativeTextGovernedStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeTextGovernedStorage")
            .finish_non_exhaustive()
    }
}
impl NativeTextGovernedStorage {
    /// New explicit configured session only; no fallback to a less capable
    /// opener on error. Complete declaration debt resumes exactly; an aborted
    /// first claim can redeclare only on the independently pristine baseline.
    pub fn open(
        dir: impl AsRef<std::path::Path>,
        secret: &[u8],
        rng: &mut impl CryptoRngCore,
        current: crate::store::V2CurrentLayoutPolicy,
    ) -> Result<Self, AppError> {
        let policy = Policy::new(
            CriticalCatalogReadPool::new(super::super::lifecycle::preparation::PREPARATION_FEE)?,
            Policy::required_persistent_capacity(),
        )?;
        Ok(Self {
            joined_declaration: None,
            index: upgrade_source::owning::Store::open_governed(dir, secret, rng, current, policy)?,
        })
    }
    /// Authenticated metadata only; neither zero nor a positive count is live
    /// membership, a complete projection or permission to author.
    pub fn row_count_data(&self) -> Result<usize, AppError> {
        self.index.native_row_count_data()
    }

    /// Stage the declaration on this pristine physical owner. Drop this
    /// metadata owner before opening the selected joined-aware storage.
    pub fn stage_joined_pin_layout(
        &mut self,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        self.index
            .stage_joined_pin_layout(&mut self.joined_declaration, rng)
    }

    /// Promote the declared original on this same physical owner. A refusal
    /// retains the original continuation, including a possibly promoted file.
    #[expect(
        clippy::result_large_err,
        reason = "A failed promotion returns the original declaration and all its custody."
    )]
    pub fn into_joined(
        mut self,
        rng: &mut impl CryptoRngCore,
    ) -> Result<NativeTextJoinedStorage, (Self, AppError)> {
        if let Err(error) =
            self.index
                .advance_joined_pin_layout(&mut self.joined_declaration, None, rng)
        {
            return Err((self, error));
        }
        drop(self.joined_declaration.take());
        Ok(NativeTextJoinedStorage {
            pending: None,
            recovered_tcp: None,
            index: self.index,
        })
    }
}

/// Explicit custody of the joined-pin catalog and its existing native leaves.
/// Opening this metadata owner cannot install a joined MLS runtime.
pub struct NativeTextJoinedStorage {
    pending: Option<JoinedPendingIo>,
    recovered_tcp: Option<Arc<super::join_tcp::Transport>>,
    index: upgrade_source::owning::Store,
}

impl NativeTextJoinedStorage {
    #[cfg(test)]
    pub(in crate::store::critical_namespaces::native_registry) fn actual_store_for_test(
        &self,
    ) -> Result<&crate::store::ServerStore, AppError> {
        self.index.actual_store_for_test()
    }

    fn opening_policy() -> Result<Policy, AppError> {
        Policy::new(
            CriticalCatalogReadPool::new(super::super::lifecycle::preparation::PREPARATION_FEE)?,
            Policy::required_persistent_capacity(),
        )
    }

    #[cfg(test)]
    pub(in crate::store) fn open_selected_shared_history(
        dir: impl AsRef<std::path::Path>,
        secret: &[u8],
        rng: &mut impl CryptoRngCore,
        current: crate::store::V2CurrentLayoutPolicy,
    ) -> Result<Self, AppError> {
        Ok(Self {
            pending: None,
            recovered_tcp: None,
            index: upgrade_source::owning::Store::open_selected_shared_history(
                dir,
                secret,
                rng,
                current,
                Self::opening_policy()?,
            )?,
        })
    }

    pub fn open(
        dir: impl AsRef<std::path::Path>,
        secret: &[u8],
        rng: &mut impl CryptoRngCore,
        current: crate::store::V2CurrentLayoutPolicy,
    ) -> Result<Self, AppError> {
        Ok(Self {
            pending: None,
            recovered_tcp: None,
            index: upgrade_source::owning::Store::open_joined(
                dir,
                secret,
                rng,
                current,
                Self::opening_policy()?,
            )?,
        })
    }

    /// Reopen once for native startup. An unpublished native Pending is
    /// eligible only while the selected vault has neither Welcome file.
    pub fn open_for_native_startup(
        directory: &std::path::Path,
        secret: &[u8],
        rng: &mut impl CryptoRngCore,
        current: crate::store::V2CurrentLayoutPolicy,
    ) -> Result<Self, AppError> {
        let opened = Self {
            pending: None,
            recovered_tcp: None,
            index: upgrade_source::owning::Store::open_native_startup(
                directory,
                secret,
                rng,
                current,
                Self::opening_policy()?,
            )?,
        };
        // Surviving rejoin evidence belongs only to the previously
        // registered joined vault. It cannot become an empty or founder
        // startup if some older catalog files are restored.
        if opened.index.has_rejoin_evidence()?
            && opened.recovered_join_registration_data()?.is_none()
        {
            return Err(refused());
        }
        // A catalog cannot turn an empty or joined registry into a founder.
        // The original saved row, pinned source and reservation must agree.
        opened.index.require_review_hold_founder()?;
        Ok(opened)
    }

    pub fn row_count_data(&self) -> Result<usize, AppError> {
        self.index.native_row_count_data()
    }

    /// Upgrade a recovered founder's existing shared-history vault before
    /// publishing it. The exact selected catalog requires a fresh opener,
    /// so a successful change must drop this old startup owner.
    pub fn select_review_storage_before_founder_startup(
        &self,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<bool, AppError> {
        if self.pending.is_some() || self.recovered_tcp.is_some() {
            return Err(refused());
        }
        if self.row_count_data()? != 1 || self.recovered_join_registration_data()?.is_some() {
            return Ok(false);
        }
        self.index.select_review_storage_before_founder_startup(rng)
    }

    /// Provision the recipient's private journal before exposing a recovered
    /// first joined source. The caller must reopen after a successful write.
    pub fn select_review_storage_before_joined_startup(
        &self,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<bool, AppError> {
        if self.pending.is_some() || self.recovered_tcp.is_some() {
            return Err(refused());
        }
        if self.row_count_data()? != 1 || self.recovered_join_registration_data()?.is_none() {
            return Ok(false);
        }
        self.index.select_review_storage_before_joined_startup(rng)
    }

    /// Mount the original cold selection as an empty native vault only if
    /// both its registry and joined pin catalog are proven empty.
    pub fn into_idle_native_vault(
        self,
        session: (Arc<AtomicU64>, Arc<AtomicU64>, Arc<AtomicBool>),
        budget: crate::store::NativeUiInvocationBudget,
    ) -> Result<crate::store::NativeTextVault, AppError> {
        if self.pending.is_some() || self.recovered_tcp.is_some() || self.row_count_data()? != 0 {
            return Err(refused());
        }
        if self.index.has_rejoin_evidence()? {
            return Err(refused());
        }
        self.index.require_empty_mounted_joined_pins()?;
        crate::store::NativeTextVault::from_recovered_joined_index(self.index, session, budget)
    }

    /// Preserve the original covered Pending when there is still no native
    /// row or Welcome entry. The next checked Join invocation must identify
    /// the same accepted invite and request before restoring private custody.
    pub fn into_native_vault_without_registered_row(
        self,
        session: (Arc<AtomicU64>, Arc<AtomicU64>, Arc<AtomicBool>),
        budget: crate::store::NativeUiInvocationBudget,
    ) -> Result<crate::store::NativeTextVault, AppError> {
        if self.pending.is_some() || self.recovered_tcp.is_some() || self.row_count_data()? != 0 {
            return Err(refused());
        }
        if self.index.has_rejoin_evidence()? {
            return Err(refused());
        }
        self.index.require_native_joined_without_row()?;
        crate::store::NativeTextVault::from_recovered_joined_index(self.index, session, budget)
    }

    /// Historical pin context from the current mounted index.
    pub fn joined_pin_context_data(
        &self,
        anchor: &catcoms_replication::chat::join_bindings::GenesisAnchorBindingData,
    ) -> Result<Option<catcoms_sync::V2PendingJoinContextData>, AppError> {
        self.index.selected_joined_pin_context_data(anchor)
    }

    /// Prepare and cover the first native request only after the caller's
    /// accepted anchor agrees with the signed invite and the mounted index.
    /// An exact retry reuses the original provider, request and covered pin.
    /// The trusted caller provides its original session cancellation and time.
    pub fn cover_accepted_first_join(
        &mut self,
        invite: &crate::store::NativeTextInvitePacketData<'_>,
        accepted: &GenesisAnchorBindingData,
        token: [u8; 32],
        now_ms: u64,
        negative: &RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        self.index.cover_accepted_first_join(
            &mut self.pending,
            AcceptedFirstJoin {
                invite,
                anchor: accepted,
                token,
                now_ms,
                transport: AcceptedJoinTransport::External,
            },
            || (!negative.is_cancelled()).then_some(()).ok_or_else(refused),
            rng,
        )?;
        Ok(())
    }

    /// Cover one original request and the first pin insertion under this
    /// selected mount. The caller retains this owner across a write refusal.
    pub fn stage_first_join(
        &mut self,
        source: PreparedV2JoinRequest,
        invite: &SignedInviteBindingData,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        if self.pending.is_some() {
            return Err(refused());
        }
        self.pending = Some(self.index.prepare_joined_pending(source, invite, rng)?);
        self.retry_first_join()
    }

    pub fn retry_first_join(&mut self) -> Result<(), AppError> {
        self.index
            .stage_joined_pending(self.pending.as_mut().ok_or_else(refused)?)
    }

    pub fn select_first_join(&mut self) -> Result<(), AppError> {
        self.index
            .install_joined_pending(self.pending.as_mut().ok_or_else(refused)?)
    }

    /// The original signed request is available after its selected working
    /// index and Pending record have been covered.
    pub fn first_join_request_data(&self) -> Result<&[u8], AppError> {
        self.index
            .joined_request_data(self.pending.as_ref().ok_or_else(refused)?)
    }

    /// Save possible entry before invoking MLS on the original private
    /// package. Exact retries retain the same delivery and completed outcome.
    pub fn finish_first_join(
        &mut self,
        delivery: &[u8],
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        self.index.finish_joined_pending(
            self.pending.as_mut().ok_or_else(refused)?,
            delivery,
            rng,
            None,
        )
    }

    /// The actual original warm selection after the complete covered Welcome.
    /// Returns historical head and epoch data under the mounted pin and files.
    pub fn warm_join_outcome_data(
        &self,
    ) -> Result<Option<(catcoms_replication::chat::owner_header::OwnerHeaderId, u64)>, AppError>
    {
        self.index
            .warm_join_outcome_data(self.pending.as_ref().ok_or_else(refused)?)
    }

    /// Exchange the exact covered request with the invited peer over an
    /// existing authenticated connection, then cover the verified Welcome.
    pub async fn exchange_first_join(
        &mut self,
        invite: &crate::store::NativeTextInvitePacketData<'_>,
        transport: &impl MeshTransport,
        negative: RequestCancellation,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        let request = self.require_exact_invite(invite)?;
        let delivery = self
            .receive_original_join(request, invite, transport, negative)
            .await?;
        self.index.finish_joined_pending(
            self.pending.as_mut().ok_or_else(refused)?,
            &delivery.bytes,
            rng,
            Some(&delivery.scope),
        )
    }

    /// Reissue the authenticated original request from an independently
    /// reopened vault whose Pending precedes any local Welcome entry. This
    /// only returns verified delivery data and owns no Welcome execution grant.
    pub async fn replay_recovered_first_delivery(
        &self,
        invite: &crate::store::NativeTextInvitePacketData<'_>,
        transport: &impl MeshTransport,
        negative: RequestCancellation,
    ) -> Result<bytes::Bytes, AppError> {
        if self.pending.is_some() || negative.is_cancelled() {
            return Err(refused());
        }
        let (_, _request_fee) = self
            .index
            .joined_exchange_resources(MAX_V2_JOIN_REQUEST_PACKET_BYTES + 4096)?;
        let mut request =
            zeroize::Zeroizing::new(Vec::with_capacity(MAX_V2_JOIN_REQUEST_PACKET_BYTES));
        self.index.copy_recovered_join_request(&mut request)?;
        Self::require_invite_bytes(&request, invite)?;
        let reply = self
            .receive_original_join(&request, invite, transport, negative)
            .await?;
        self.index
            .validate_recovered_join_delivery(&request, &reply.bytes)?;
        if reply.scope.is_cancelled() {
            return Err(refused());
        }
        Ok(bytes::Bytes::from_owner(reply))
    }

    async fn receive_original_join(
        &self,
        request: &[u8],
        invite: &crate::store::NativeTextInvitePacketData<'_>,
        transport: &impl MeshTransport,
        negative: RequestCancellation,
    ) -> Result<ConnectedJoinDelivery, AppError> {
        let (mount, fee) = self
            .index
            .joined_exchange_resources(connected_join_invoice(request.len()))?;
        receive_connected_join(
            request,
            invite.connected_peer_data(),
            transport,
            negative,
            mount,
            Arc::new(JoinFramePayment { _fee: fee }),
        )
        .await
    }

    fn require_exact_invite(
        &self,
        invite: &crate::store::NativeTextInvitePacketData<'_>,
    ) -> Result<&[u8], AppError> {
        let request = self.first_join_request_data()?;
        Self::require_invite_bytes(request, invite)?;
        Ok(request)
    }

    pub(super) fn require_invite_bytes(
        request: &[u8],
        invite: &crate::store::NativeTextInvitePacketData<'_>,
    ) -> Result<(), AppError> {
        let packet = V2JoinRequestPacketData::decode(request).map_err(|_| refused())?;
        if packet.founder_add_data().invite.encode() != invite.binding().encode() {
            return Err(refused());
        }
        Ok(())
    }

    /// Historical outcome installed by the cold storage opener.
    pub fn recovered_join_outcome_data(
        &self,
    ) -> Result<Option<(catcoms_replication::chat::owner_header::OwnerHeaderId, u64)>, AppError>
    {
        self.index.recovered_join_outcome_data()
    }

    /// Reference to the exact saved request for diagnostics and later admission.
    pub fn pending_join_data(
        &self,
    ) -> Result<Option<([u8; 32], V2PendingJoinContextData)>, AppError> {
        self.index.selected_pending_join_data()
    }

    /// Public identity and reserved port stored by the original native
    /// Pending. Opening these historical facts starts no transport actor.
    pub fn recovered_join_transport_data(
        &self,
    ) -> Result<Option<(catcoms_rt::PeerId, u16)>, AppError> {
        self.index.recovered_join_transport_data()
    }

    /// Historical native registration fields independently proved from the
    /// original Pending, Welcome and selected governing replay after reopen.
    pub fn recovered_join_registration_data(
        &self,
    ) -> Result<Option<crate::store::NativeTextJoinedRegistrationData>, AppError> {
        self.index.recovered_join_registration_data()
    }
}

pub(super) fn connected_join_invoice(request_bytes: usize) -> usize {
    4 * (request_bytes + 1 + CHAT_V2_JOIN_RESPONSE_MAX_BYTES) + 4096
}

pub(super) async fn receive_connected_join(
    request: &[u8],
    target: catcoms_rt::PeerId,
    transport: &impl MeshTransport,
    negative: RequestCancellation,
    mount: RequestCancellation,
    fee: Arc<JoinFramePayment>,
) -> Result<ConnectedJoinDelivery, AppError> {
    let connection = transport.connection_lease(target).ok_or_else(refused)?;
    let authority = Arc::new(JoinConnectionAuthority {
        connection,
        mount,
        caller: negative.clone(),
    });
    if !authority.authorized() {
        return Err(refused());
    }
    let negative = negative
        .with_authority(authority.clone())
        .retain_custody(fee);
    let mut wire = Vec::with_capacity(1 + request.len());
    wire.push(CHAT_V2_JOIN_KIND);
    wire.extend_from_slice(request);
    let delivery = transport
        .request_connected_cancellable(
            target,
            JOIN_PROTOCOL,
            bytes::Bytes::from(wire),
            negative.clone(),
        )
        .await
        .map_err(|_| refused())?;
    if !authority.authorized() || delivery.len() > CHAT_V2_JOIN_RESPONSE_MAX_BYTES {
        return Err(refused());
    }
    Ok(ConnectedJoinDelivery {
        bytes: delivery,
        scope: negative,
    })
}
