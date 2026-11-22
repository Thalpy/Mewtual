//! Authenticated first-pin and private JoinRequest transaction.
//! The mounted owner covers this exact output before any request can leave.
use super::*;
use catcoms_sync::{
    DirectJoinLimits, PreparedV2JoinRequest, RecoveredV2PendingJoinSource,
    SealedV2PendingJoinSource, V2JoinRequestPacketData, V2PendingJoinContextData,
    MAX_V2_JOIN_REQUEST_PACKET_BYTES,
};
use index::{JoinedPinIndex, MAX_FRAME as MAX_INDEX_FRAME};

const TAG: &[u8] = b"catcoms/private-chat-v2-pinned-pending-join/v1\0";
const VERSION: u16 = 1;
const KEY_DOMAIN: &str = "catcoms/private-chat-v2-pinned-pending-join-key/v1";
const MAX_SOURCE: usize = 512 * 1024 + MAX_V2_JOIN_REQUEST_PACKET_BYTES + 1024;
const HEAD: usize = 4 + TAG.len() + 2 + 4 * 36 + 4 + 24 + 4 * 4;
pub(in crate::store) const MAX_FRAME: usize =
    HEAD + 2 * MAX_INDEX_FRAME + MAX_SOURCE + MAX_JOIN_BINDING_BYTES + 40;

pub(in crate::store) struct PreparedPinnedPending {
    source: PreparedV2JoinRequest,
    context: V2PendingJoinContextData,
    invite: Vec<u8>,
    before: Zeroizing<Vec<u8>>,
    next: Zeroizing<Vec<u8>>,
    sealed: Option<Zeroizing<Vec<u8>>>,
    key_id: [u8; 32],
}

impl PreparedPinnedPending {
    pub(in crate::store) fn request_bytes_data(&self) -> &[u8] {
        self.source.request_bytes_data()
    }

    pub(in crate::store) fn context_data(&self) -> V2PendingJoinContextData {
        self.context
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
        transport: T,
        rng: R,
        clock: Box<dyn catcoms_rt::Clock + Send>,
    ) -> Result<catcoms_sync::ClosedV2JoinedNode<T, R>, AppError> {
        self.source
            .take_closed_warm_node(prefix, transport, rng, clock)
            .map_err(|_| refused())
    }

    pub(in crate::store) fn validate_delivery_data(&self, delivery: &[u8]) -> Result<(), AppError> {
        self.source
            .validate_first_join_delivery_data(delivery)
            .map_err(|_| refused())
    }

    pub(in crate::store) fn finish_delivery(
        &mut self,
        key: &[u8; 32],
        delivery: &[u8],
        rng: &mut impl CryptoRngCore,
        negative: Option<&catcoms_rt::RequestCancellation>,
    ) -> Result<&catcoms_sync::SealedV2JoinedOutcome, AppError> {
        match negative {
            Some(negative) => self
                .source
                .finish_first_join_delivery_cancellable(delivery, negative),
            None => self.source.finish_first_join_delivery(delivery),
        }
        .map_err(|_| refused())?;
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
        original_pending: [u8; 32],
        rng: &mut impl CryptoRngCore,
    ) -> Result<Self, AppError> {
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
                HEAD + self.before.len() + self.next.len() + self.invite.len() + bytes.len(),
            );
            let bad = |_| refused();
            e.put_bytes(TAG).map_err(bad)?;
            e.put_u16(VERSION);
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
}

impl RecoveredPinnedPending {
    pub(in crate::store) fn open(
        key: &[u8; 32],
        layout: CurrentLayoutData,
        raw: &[u8],
        limits: DirectJoinLimits,
    ) -> Result<Self, AppError> {
        if !(HEAD + 40..=MAX_FRAME).contains(&raw.len()) {
            return Err(refused());
        }
        let journal_key = Zeroizing::new(blake3::derive_key(KEY_DOMAIN, key));
        let plain = Zeroizing::new(unseal(&journal_key, &unframe(raw)?)?);
        let mut d = Decoder::new(&plain);
        let bad = |_| refused();
        if d.get_bytes().map_err(bad)? != TAG
            || d.get_u16().map_err(bad)? != VERSION
            || d.get_bytes().map_err(bad)? != layout.vault
            || d.get_bytes().map_err(bad)? != layout.root
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
        let source =
            SealedV2PendingJoinSource::from_encrypted_parts(nonce, bytes).map_err(|_| refused())?;
        let pending = original_pin.open_pending_source(key, &source, limits)?;
        Ok(Self {
            original: Zeroizing::new(original.to_vec()),
            candidate: Zeroizing::new(candidate.to_vec()),
            pending,
            context,
        })
    }

    pub(in crate::store) fn index_frames(&self) -> (&[u8], &[u8]) {
        (&self.original, &self.candidate)
    }

    pub(in crate::store) fn pending_data(
        &self,
    ) -> (&RecoveredV2PendingJoinSource, V2PendingJoinContextData) {
        (&self.pending, self.context)
    }
}
