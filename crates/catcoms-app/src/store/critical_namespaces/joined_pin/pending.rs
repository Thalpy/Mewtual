//! Authenticated first-pin and private JoinRequest transaction.
//! The mounted owner covers this exact output before any request can leave.
use super::*;
use crate::store::persistence::v2_member_outbox::{
    MemberOutboxContextData, MemberOutboxContextSpec,
};
use catcoms_replication::chat::{
    owner_facts::OwnerFactId,
    owner_header::{OwnerHeaderId, OwnerTenureId},
};
use catcoms_sync::{
    DirectJoinLimits, NativeV2JoinKeyEntry, PreparedV2JoinRequest, RecoveredV2PendingJoinSource,
    SealedV2PendingJoinSource, V2JoinRequestPacketData, V2PendingJoinContextData,
    MAX_V2_JOIN_REQUEST_PACKET_BYTES,
};
use index::{JoinedPinIndex, MAX_FRAME as MAX_INDEX_FRAME};

const TAG: &[u8] = b"catcoms/private-chat-v2-pinned-pending-join/v1\0";
const VERSION: u16 = 1;
const NATIVE_TRANSPORT_VERSION: u16 = 2;
const KEY_DOMAIN: &str = "catcoms/private-chat-v2-pinned-pending-join-key/v1";
const MAX_SOURCE: usize = 512 * 1024 + MAX_V2_JOIN_REQUEST_PACKET_BYTES + 1024;
const HEAD: usize = 4 + TAG.len() + 2 + 4 * 36 + 4 + 24 + 4 * 4;
pub(in crate::store) const MAX_FRAME: usize =
    HEAD + 2 * MAX_INDEX_FRAME + MAX_SOURCE + MAX_JOIN_BINDING_BYTES + 40 + 36 + 2;

/// Historical registration facts read from the exact selected Pending and
/// Welcome. This carries no source, key, runtime or native write authority.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct NativeTextJoinedRegistrationData {
    pub(in crate::store) context: V2PendingJoinContextData,
    pub(in crate::store) group: [u8; 16],
    pub(in crate::store) profile: [u8; 32],
    pub(in crate::store) signer: [u8; 32],
    pub(in crate::store) invocation: [u8; 32],
    pub(in crate::store) invite: [u8; 32],
    pub(in crate::store) request: [u8; 32],
    pub(in crate::store) transport: (catcoms_rt::PeerId, u16, [u8; 32]),
    pub(in crate::store) selected: SelectedJoinData,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store) struct SelectedJoinData {
    pub(in crate::store) governing_add: OwnerHeaderId,
    pub(in crate::store) own_admission: OwnerFactId,
    pub(in crate::store) state: [u8; 32],
    pub(in crate::store) epoch: u64,
    pub(in crate::store) profile: OwnerFactId,
    pub(in crate::store) initial_tenure: OwnerTenureId,
    pub(in crate::store) profile_creation: [u8; 32],
}

impl std::fmt::Debug for NativeTextJoinedRegistrationData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeTextJoinedRegistrationData")
            .finish_non_exhaustive()
    }
}

impl NativeTextJoinedRegistrationData {
    pub(in crate::store) fn from_selection(
        request: &[u8],
        context: V2PendingJoinContextData,
        transport: (catcoms_rt::PeerId, u16, [u8; 32]),
        selected: SelectedJoinData,
    ) -> Result<Self, AppError> {
        let packet = V2JoinRequestPacketData::decode(request).map_err(|_| refused())?;
        let original = packet.founder_add_data();
        let anchor = &original.invite.spec_data().anchor;
        let spec = original.request.spec_data();
        if selected.profile.as_bytes() != &anchor.profile {
            return Err(refused());
        }
        Ok(Self {
            context,
            group: anchor.group.as_slice().try_into().map_err(|_| refused())?,
            profile: anchor.profile,
            signer: spec.joiner,
            invocation: spec.request_token,
            invite: original.invite.id_data(),
            request: *blake3::hash(request).as_bytes(),
            transport,
            selected,
        })
    }

    pub fn selected_data(&self) -> (OwnerHeaderId, OwnerFactId, u64) {
        (
            self.selected.governing_add,
            self.selected.own_admission,
            self.selected.epoch,
        )
    }

    pub fn transport_data(&self) -> (catcoms_rt::PeerId, u16) {
        (self.transport.0, self.transport.1)
    }

    pub(in crate::store) fn member_outbox_context_data(
        &self,
    ) -> Result<MemberOutboxContextData, AppError> {
        MemberOutboxContextData::from_spec(MemberOutboxContextSpec {
            vault: self.context.vault,
            layout: self.context.root,
            pin: self.context.pin,
            profile: self.selected.profile,
            tenure: self.selected.initial_tenure,
            group_nonce: self.selected.profile_creation,
            device: self.signer,
            group: &self.group,
        })
    }
}

/// The first native transport reservation, bound to the original Pending.
/// Storage readers may view public facts without creating a transport.
pub(in crate::store) struct JoinedTransportReservation {
    seed: Zeroizing<[u8; 32]>,
    port: u16,
}

impl JoinedTransportReservation {
    pub(in crate::store) fn generate(rng: &mut impl CryptoRngCore) -> Result<Self, AppError> {
        let mut seed = Zeroizing::new([0; 32]);
        rng.try_fill_bytes(seed.as_mut()).map_err(|_| refused())?;
        let port = crate::store::ServerNet::derived_port_for_seed(&seed);
        Self::new(seed, port)
    }

    fn new(seed: Zeroizing<[u8; 32]>, port: u16) -> Result<Self, AppError> {
        if *seed == [0; 32]
            || port == 0
            || port != crate::store::ServerNet::derived_port_for_seed(&seed)
        {
            return Err(refused());
        }
        Ok(Self { seed, port })
    }

    pub(in crate::store) fn identity_data(
        &self,
    ) -> Result<(catcoms_rt::PeerId, u16, [u8; 32]), AppError> {
        let identity =
            catcoms_net::seeded_transport_identity_data(&self.seed).map_err(|_| refused())?;
        Ok((
            identity.phase0_peer_data(),
            self.port,
            identity.commitment_data(),
        ))
    }
}

pub(in crate::store) struct PreparedPinnedPending {
    source: PreparedV2JoinRequest,
    context: V2PendingJoinContextData,
    invite: Vec<u8>,
    before: Zeroizing<Vec<u8>>,
    next: Zeroizing<Vec<u8>>,
    sealed: Option<Zeroizing<Vec<u8>>>,
    transport: Option<JoinedTransportReservation>,
    key_id: [u8; 32],
}

impl PreparedPinnedPending {
    pub(in crate::store) fn native_transport_seed(&self) -> Result<Zeroizing<[u8; 32]>, AppError> {
        self.transport
            .as_ref()
            .map(|original| original.seed.clone())
            .ok_or_else(refused)
    }

    pub(in crate::store) fn request_bytes_data(&self) -> &[u8] {
        self.source.request_bytes_data()
    }

    pub(in crate::store) fn context_data(&self) -> V2PendingJoinContextData {
        self.context
    }

    pub(in crate::store) fn transport_data(
        &self,
    ) -> Result<Option<(catcoms_rt::PeerId, u16, [u8; 32])>, AppError> {
        self.transport
            .as_ref()
            .map(JoinedTransportReservation::identity_data)
            .transpose()
    }

    pub(in crate::store) fn group_data(&self) -> Result<Vec<u8>, AppError> {
        let invite = SignedInviteBindingData::decode(&self.invite).map_err(|_| refused())?;
        Ok(invite.spec_data().anchor.group.clone())
    }

    pub(in crate::store) fn prepare_initial_prefix(
        &mut self,
        delivery: &[u8],
        group: &catcoms_replication::chat::ProjectionGroupView,
        origin: catcoms_replication::chat::V2ProjectionOrigin,
    ) -> Result<
        (
            catcoms_sync::PreparedV2JoinedPrefix,
            catcoms_replication::chat::V2HostSelectionExpectation,
        ),
        AppError,
    > {
        self.source
            .prepare_initial_prefix(delivery, group, origin)
            .map_err(|_| refused())
    }

    pub(in crate::store) fn take_closed_warm_node<
        T: catcoms_rt::MeshTransport,
        R: catcoms_rt::CryptoRngCore,
    >(
        &mut self,
        prefix: &catcoms_sync::PreparedV2JoinedPrefix,
        inputs: &mut Option<catcoms_sync::V2JoinedRuntimeInputs<T, R>>,
    ) -> Result<catcoms_sync::ClosedV2JoinedNode<T, R>, AppError> {
        self.source
            .take_closed_warm_node(prefix, inputs)
            .map_err(|_| refused())
    }

    pub(in crate::store) fn validate_delivery_data(&self, delivery: &[u8]) -> Result<(), AppError> {
        self.source
            .validate_first_join_delivery_data(delivery)
            .map_err(|_| refused())
    }

    pub(in crate::store) fn finish_delivery(
        &mut self,
        delivery: &[u8],
        negative: Option<&catcoms_rt::RequestCancellation>,
        key: Option<NativeV2JoinKeyEntry<'_>>,
    ) -> Result<(), AppError> {
        match (negative, key) {
            (Some(negative), Some(key)) => self
                .source
                .finish_native_first_join_delivery(delivery, negative, key),
            (Some(negative), None) => self
                .source
                .finish_first_join_delivery_cancellable(delivery, negative),
            (None, None) => self.source.finish_first_join_delivery(delivery),
            (None, Some(_)) => return Err(refused()),
        }
        .map(|_| ())
        .map_err(|_| refused())
    }

    pub(in crate::store) fn seal_outcome(
        &mut self,
        key: &[u8; 32],
        rng: &mut impl CryptoRngCore,
    ) -> Result<&catcoms_sync::SealedV2JoinedOutcome, AppError> {
        self.source
            .seal_joined_outcome(key, self.context, rng)
            .map_err(|_| refused())
    }

    pub(in crate::store) fn prepare(
        source: PreparedV2JoinRequest,
        selected_index: &[u8],
        accepted: &SignedInviteBindingData,
        key: &[u8; 32],
        layout: CurrentLayoutData,
        identity: ([u8; 32], Option<JoinedTransportReservation>),
        rng: &mut impl CryptoRngCore,
    ) -> Result<Self, AppError> {
        let (original_pending, transport) = identity;
        let packet =
            V2JoinRequestPacketData::decode(source.request_bytes_data()).map_err(|_| refused())?;
        let invite = accepted.encode();
        if packet.founder_add_data().invite.encode() != invite {
            return Err(refused());
        }
        let before = JoinedPinIndex::open(key, layout, selected_index)?;
        let next = before.prepare_insert(key, accepted, original_pending, rng)?;
        let candidate = JoinedPinIndex::open(key, layout, &next)?;
        before.require_successor(&candidate)?;
        let context = candidate
            .pending_context_data(&accepted.spec_data().anchor)?
            .ok_or_else(refused)?;
        if context.original_pending != original_pending {
            return Err(refused());
        }
        Ok(Self {
            source,
            context,
            invite,
            before: Zeroizing::new(selected_index.to_vec()),
            next,
            sealed: None,
            transport,
            key_id: *blake3::keyed_hash(key, TAG).as_bytes(),
        })
    }

    pub(in crate::store) fn seal(
        &mut self,
        key: &[u8; 32],
        rng: &mut impl CryptoRngCore,
    ) -> Result<&[u8], AppError> {
        if *blake3::keyed_hash(key, TAG).as_bytes() != self.key_id {
            return Err(refused());
        }
        if self.sealed.is_none() {
            let source = self
                .source
                .seal_pending_source(key, self.context, rng)
                .map_err(|_| refused())?;
            let (nonce, bytes) = source.encrypted_parts();
            if bytes.len() > MAX_SOURCE {
                return Err(refused());
            }
            let mut e = Encoder::with_capacity(
                HEAD + self.before.len()
                    + self.next.len()
                    + self.invite.len()
                    + bytes.len()
                    + self.transport.as_ref().map_or(0, |_| 36 + 2),
            );
            let bad = |_| refused();
            e.put_bytes(TAG).map_err(bad)?;
            e.put_u16(if self.transport.is_some() {
                NATIVE_TRANSPORT_VERSION
            } else {
                VERSION
            });
            for field in [
                self.context.vault,
                self.context.root,
                self.context.original_pending,
                self.context.pin,
            ] {
                e.put_bytes(&field).map_err(bad)?;
            }
            for field in [
                self.before.as_slice(),
                self.next.as_slice(),
                self.invite.as_slice(),
                nonce.as_slice(),
                bytes,
            ] {
                e.put_bytes(field).map_err(bad)?;
            }
            if let Some(transport) = &self.transport {
                e.put_bytes(transport.seed.as_slice()).map_err(bad)?;
                e.put_u16(transport.port);
            }
            let plain = Zeroizing::new(e.finish());
            let key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
            self.sealed = Some(Zeroizing::new(frame(&catcoms_crypto::seal(
                &key, &plain, rng,
            )?)));
        }
        self.sealed
            .as_deref()
            .map(Vec::as_slice)
            .ok_or_else(refused)
    }
}

pub(in crate::store) struct RecoveredPinnedPending {
    original: Zeroizing<Vec<u8>>,
    candidate: Zeroizing<Vec<u8>>,
    pending: RecoveredV2PendingJoinSource,
    context: V2PendingJoinContextData,
    transport: Option<JoinedTransportReservation>,
}

struct PinnedPendingFields {
    original: Zeroizing<Vec<u8>>,
    candidate: Zeroizing<Vec<u8>>,
    context: V2PendingJoinContextData,
    transport: Option<JoinedTransportReservation>,
    sealed: SealedV2PendingJoinSource,
    accepted: SignedInviteBindingData,
}

impl RecoveredPinnedPending {
    pub(in crate::store) fn native_transport_seed(&self) -> Result<Zeroizing<[u8; 32]>, AppError> {
        self.transport
            .as_ref()
            .map(|original| original.seed.clone())
            .ok_or_else(refused)
    }

    pub(in crate::store) fn open(
        key: &[u8; 32],
        layout: CurrentLayoutData,
        raw: &[u8],
        limits: DirectJoinLimits,
    ) -> Result<Self, AppError> {
        let fields = Self::open_fields(key, layout, raw)?;
        let pending = RecoveredV2PendingJoinSource::open(
            &fields.sealed,
            key,
            fields.context,
            &fields.accepted,
            limits,
        )
        .map_err(|_| refused())?;
        Ok(Self {
            original: fields.original,
            candidate: fields.candidate,
            pending,
            context: fields.context,
            transport: fields.transport,
        })
    }

    /// The physical caller already holds the original Pending and has
    /// checked that neither Welcome file exists under the mounted vault.
    pub(in crate::store) fn restore_native_pre_welcome(
        key: &[u8; 32],
        layout: CurrentLayoutData,
        raw: &[u8],
        limits: DirectJoinLimits,
        native: NativeV2JoinKeyEntry<'_>,
    ) -> Result<PreparedPinnedPending, AppError> {
        let fields = Self::open_fields(key, layout, raw)?;
        let transport = fields.transport.ok_or_else(refused)?;
        let source = fields
            .sealed
            .restore_native_pre_welcome(key, fields.context, &fields.accepted, limits, native)
            .map_err(|_| refused())?;
        Ok(PreparedPinnedPending {
            source,
            context: fields.context,
            invite: fields.accepted.encode(),
            before: fields.original,
            next: fields.candidate,
            sealed: Some(Zeroizing::new(raw.to_vec())),
            transport: Some(transport),
            key_id: *blake3::keyed_hash(key, TAG).as_bytes(),
        })
    }

    fn open_fields(
        key: &[u8; 32],
        layout: CurrentLayoutData,
        raw: &[u8],
    ) -> Result<PinnedPendingFields, AppError> {
        if !(HEAD + 40..=MAX_FRAME).contains(&raw.len()) {
            return Err(refused());
        }
        let journal_key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
        let plain = Zeroizing::new(unseal(&journal_key, &unframe(raw)?)?);
        let mut d = Decoder::new(&plain);
        let bad = |_| refused();
        if d.get_bytes().map_err(bad)? != TAG {
            return Err(refused());
        }
        let version = d.get_u16().map_err(bad)?;
        if d.get_bytes().map_err(bad)? != layout.vault || d.get_bytes().map_err(bad)? != layout.root
        {
            return Err(refused());
        }
        let original_pending: [u8; 32] = d
            .get_bytes()
            .map_err(bad)?
            .try_into()
            .map_err(|_| refused())?;
        let pin: [u8; 32] = d
            .get_bytes()
            .map_err(bad)?
            .try_into()
            .map_err(|_| refused())?;
        let original = d.get_bytes().map_err(bad)?;
        let candidate = d.get_bytes().map_err(bad)?;
        let accepted =
            SignedInviteBindingData::decode(d.get_bytes().map_err(bad)?).map_err(|_| refused())?;
        let nonce = d
            .get_bytes()
            .map_err(bad)?
            .try_into()
            .map_err(|_| refused())?;
        let bytes = d.get_bytes().map_err(bad)?;
        let transport = match version {
            VERSION => None,
            NATIVE_TRANSPORT_VERSION => {
                let seed = Zeroizing::new(
                    d.get_bytes()
                        .map_err(bad)?
                        .try_into()
                        .map_err(|_| refused())?,
                );
                Some(JoinedTransportReservation::new(
                    seed,
                    d.get_u16().map_err(bad)?,
                )?)
            }
            _ => return Err(refused()),
        };
        d.finish().map_err(bad)?;
        if bytes.len() > MAX_SOURCE {
            return Err(refused());
        }
        let before = JoinedPinIndex::open(key, layout, original)?;
        let after = JoinedPinIndex::open(key, layout, candidate)?;
        before.require_successor(&after)?;
        let anchor = &accepted.spec_data().anchor;
        if before.lookup(anchor)?.is_some() {
            return Err(refused());
        }
        let original_pin = after.lookup(anchor)?.ok_or_else(refused)?;
        let context = original_pin.pending_context_data();
        if context.original_pending != original_pending
            || context.pin != pin
            || original_pin.accepted_invite.encode() != accepted.encode()
        {
            return Err(refused());
        }
        Ok(PinnedPendingFields {
            original: Zeroizing::new(original.to_vec()),
            candidate: Zeroizing::new(candidate.to_vec()),
            context,
            transport,
            sealed: SealedV2PendingJoinSource::from_encrypted_parts(nonce, bytes)
                .map_err(|_| refused())?,
            accepted,
        })
    }

    pub(in crate::store) fn index_frames(&self) -> (&[u8], &[u8]) {
        (&self.original, &self.candidate)
    }

    pub(in crate::store) fn transport_data(
        &self,
    ) -> Result<Option<(catcoms_rt::PeerId, u16, [u8; 32])>, AppError> {
        self.transport
            .as_ref()
            .map(JoinedTransportReservation::identity_data)
            .transpose()
    }

    pub(in crate::store) fn pending_data(
        &self,
    ) -> (&RecoveredV2PendingJoinSource, V2PendingJoinContextData) {
        (&self.pending, self.context)
    }
}
