//! Explicit governed cold storage, with no signer/Source/native command or
//! facade conversion. Operational cold authoring needs its separate owner.
use super::super::lifecycle::preparation::Policy;
use super::*;
use crate::store::persistence::governance_layout::joined::{JoinedPendingIo, JoinedPinDeclaration};
use catcoms_replication::chat::join_bindings::SignedInviteBindingData;
use catcoms_rt::{
    ConnectionLease, CryptoRngCore, MeshTransport, ProtocolId, RequestAuthority,
    RequestCancellation, CHAT_V2_JOIN_KIND, CHAT_V2_JOIN_REQUEST_MAX_BYTES,
    CHAT_V2_JOIN_RESPONSE_MAX_BYTES,
};
use catcoms_sync::{
    PreparedV2JoinRequest, V2JoinRequestPacketData, V2PendingJoinContextData,
    MAX_V2_FIRST_JOIN_DELIVERY_BYTES, MAX_V2_JOIN_REQUEST_PACKET_BYTES,
};
mod runtime;
pub use runtime::NativeTextJoinedRuntime;

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

struct JoinFramePayment {
    _fee: CatalogReadReservation,
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
            index: self.index,
        })
    }
}

/// Explicit custody of the joined-pin catalog and its existing native leaves.
/// Opening this metadata owner cannot install a joined MLS runtime.
pub struct NativeTextJoinedStorage {
    pending: Option<JoinedPendingIo>,
    index: upgrade_source::owning::Store,
}

impl NativeTextJoinedStorage {
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
            pending: None,
            index: upgrade_source::owning::Store::open_joined(dir, secret, rng, current, policy)?,
        })
    }

    pub fn row_count_data(&self) -> Result<usize, AppError> {
        self.index.native_row_count_data()
    }

    /// Historical pin context from the current mounted index.
    pub fn joined_pin_context_data(
        &self,
        anchor: &catcoms_replication::chat::join_bindings::GenesisAnchorBindingData,
    ) -> Result<Option<catcoms_sync::V2PendingJoinContextData>, AppError> {
        self.index.selected_joined_pin_context_data(anchor)
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
        let target = invite.connected_peer_data();
        let connection = transport.connection_lease(target).ok_or_else(refused)?;
        let (mount, fee) = self.index.joined_exchange_resources(
            4 * (request.len() + 1 + CHAT_V2_JOIN_RESPONSE_MAX_BYTES) + 4096,
        )?;
        let fee = Arc::new(JoinFramePayment { _fee: fee });
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
            .retain_custody(fee.clone());
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
        if !authority.authorized() || delivery.len() > MAX_V2_FIRST_JOIN_DELIVERY_BYTES {
            return Err(refused());
        }
        self.index.finish_joined_pending(
            self.pending.as_mut().ok_or_else(refused)?,
            &delivery,
            rng,
            Some(&negative),
        )
    }

    fn require_exact_invite(
        &self,
        invite: &crate::store::NativeTextInvitePacketData<'_>,
    ) -> Result<&[u8], AppError> {
        let request = self.first_join_request_data()?;
        let packet = V2JoinRequestPacketData::decode(request).map_err(|_| refused())?;
        if packet.founder_add_data().invite.encode() != invite.binding().encode() {
            return Err(refused());
        }
        Ok(request)
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
}
