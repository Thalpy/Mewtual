//! An epoch-bound member advertisement of an already owned TCP listener.
use super::*;
use catcoms_replication::chat::{owner_header::OwnerBodyId, V2LiveMemberObservation};

const DOMAIN: &[u8] = b"mewtual/chat/v2/member-contact/v1\0";
const PEER_BYTES: usize = 38;
const INLINE_ED25519: &[u8; 6] = &[0, 36, 8, 1, 18, 32];
const PROOF_BYTES: usize = 32 + 32 + 2 + 64;
const PREIMAGE_BYTES: usize = DOMAIN.len() + 3 * 32 + 8 + PEER_BYTES + 2;
pub const V2_MEMBER_CONTACT_BYTES: usize = PREIMAGE_BYTES + 64;

pub struct V2MemberContactData<'a> {
    raw: &'a [u8; V2_MEMBER_CONTACT_BYTES],
    profile: &'a [u8; 32],
    head: &'a [u8; 32],
    admission: OwnerBodyId,
    epoch: u64,
    peer: &'a [u8; PEER_BYTES],
    port: u16,
}

impl std::fmt::Debug for V2MemberContactData<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V2MemberContactData")
            .finish_non_exhaustive()
    }
}

impl<'a> V2MemberContactData<'a> {
    pub(crate) const CONNECTED_PROOF_BYTES: usize = PROOF_BYTES;

    pub fn decode(raw: &'a [u8]) -> Result<Self, SyncError> {
        let raw: &[u8; V2_MEMBER_CONTACT_BYTES] =
            raw.try_into().map_err(|_| SyncError::Malformed)?;
        let (domain, rest) = raw.split_at(DOMAIN.len());
        if domain != DOMAIN {
            return Err(SyncError::Malformed);
        }
        let (profile, rest) = rest.split_first_chunk::<32>().ok_or(SyncError::Malformed)?;
        let (head, rest) = rest.split_first_chunk::<32>().ok_or(SyncError::Malformed)?;
        let (admission, rest) = rest.split_first_chunk::<32>().ok_or(SyncError::Malformed)?;
        let (epoch, rest) = rest.split_first_chunk::<8>().ok_or(SyncError::Malformed)?;
        let (peer, rest) = rest
            .split_first_chunk::<PEER_BYTES>()
            .ok_or(SyncError::Malformed)?;
        let (port, _signature) = rest.split_first_chunk::<2>().ok_or(SyncError::Malformed)?;
        let port = u16::from_be_bytes(*port);
        if [profile, head, admission].contains(&&[0; 32]) || port == 0 || !valid_peer_encoding(peer)
        {
            return Err(SyncError::Malformed);
        }
        Ok(Self {
            raw,
            profile,
            head,
            admission: OwnerBodyId::from_bytes(*admission),
            epoch: u64::from_be_bytes(*epoch),
            peer,
            port,
        })
    }

    pub fn address_data(&self) -> (&'a [u8; PEER_BYTES], u16) {
        (self.peer, self.port)
    }

    pub fn admission_data(&self) -> OwnerBodyId {
        self.admission
    }

    /// An authenticated current group carrier already has the Profile,
    /// Header and epoch. Keep the other signed fields under its wire cap.
    pub(crate) fn connected_proof(&self) -> [u8; PROOF_BYTES] {
        let mut proof = [0; PROOF_BYTES];
        proof[..32].copy_from_slice(self.admission.as_bytes());
        proof[32..].copy_from_slice(&self.raw[DOMAIN.len() + 3 * 32 + 8 + INLINE_ED25519.len()..]);
        proof
    }

    pub(crate) fn require_connected_proof(
        verifier: &V2GeneralPrefixVerifier,
        requester: catcoms_rt::PeerId,
        proof: &[u8; PROOF_BYTES],
    ) -> Result<V2LiveMemberObservation, SyncError> {
        let replay = verifier.replay_data();
        let mut full = [0; V2_MEMBER_CONTACT_BYTES];
        let mut next = full.as_mut_slice();
        let profile = replay.profile().id();
        let header = replay.header().id();
        let epoch = replay.mls_epoch_data().to_be_bytes();
        let fields: [&[u8]; 7] = [
            DOMAIN,
            profile.as_bytes(),
            header.as_bytes(),
            &proof[..32],
            &epoch,
            INLINE_ED25519,
            &proof[32..],
        ];
        for field in fields {
            let (front, rest) = next.split_at_mut(field.len());
            front.copy_from_slice(field);
            next = rest;
        }
        let contact = V2MemberContactData::decode(&full)?;
        if blake3::hash(contact.peer).as_bytes() != requester.as_bytes() {
            return Err(SyncError::JoinRejected);
        }
        contact.verify_selected(verifier)?;
        verifier
            .observe_live_member(OwnerFactId::from_bytes(*contact.admission.as_bytes()))
            .map_err(|_| SyncError::JoinRejected)
    }

    /// Reprove the current signed holder against the actual connected peer.
    /// The caller retains its approved contact and original connection.
    pub fn require_review_holder(
        &self,
        verifier: &V2GeneralPrefixVerifier,
        connected_peer: catcoms_rt::PeerId,
    ) -> Result<OwnerFactId, SyncError> {
        if blake3::hash(self.peer).as_bytes() != connected_peer.as_bytes() {
            return Err(SyncError::JoinRejected);
        }
        self.verify_selected(verifier)?;
        let admission = OwnerFactId::from_bytes(*self.admission.as_bytes());
        verifier
            .observe_live_member(admission)
            .map_err(|_| SyncError::JoinRejected)?;
        Ok(admission)
    }

    fn verify_selected(&self, verifier: &V2GeneralPrefixVerifier) -> Result<(), SyncError> {
        let replay = verifier.replay_data();
        if replay.profile().id().as_bytes() != self.profile
            || replay.header().id().as_bytes() != self.head
            || replay.mls_epoch_data() != self.epoch
        {
            return Err(SyncError::JoinRejected);
        }
        let slot = replay.slot(self.admission).ok_or(SyncError::JoinRejected)?;
        let (preimage, signature) = self.raw.split_at(PREIMAGE_BYTES);
        if !catcoms_crypto::verify_with_public_bytes(
            &slot.image.subject,
            preimage,
            signature.try_into().map_err(|_| SyncError::Malformed)?,
        ) {
            return Err(SyncError::JoinRejected);
        }
        Ok(())
    }
}

fn valid_peer_encoding(peer: &[u8; PEER_BYTES]) -> bool {
    // The mounted TCP identity is the canonical inline libp2p Ed25519 peer.
    peer.starts_with(INLINE_ED25519) && peer[INLINE_ED25519.len()..] != [0; 32]
}

impl<T: MeshTransport, R: CryptoRngCore> ClosedV2JoinedNode<T, R> {
    pub fn bind_recovered_contact_source(
        &mut self,
        prefix: &PreparedV2JoinedPrefix,
        source: &NativeChatSourceObservation,
        cut: &NativeChatPreparedKeyCut,
    ) -> Result<(), SyncError> {
        self.bind_recovered_native_source(prefix, source, cut, NativeChatKeyPurpose::MemberContact)
    }

    pub fn verify_member_contact(
        &self,
        prefix: &PreparedV2JoinedPrefix,
        contact: &V2MemberContactData<'_>,
    ) -> Result<(), SyncError> {
        if contact.admission.as_bytes() == self.admission_id.as_bytes()
            || blake3::hash(contact.peer).as_bytes() == self.local_peer_data().as_bytes()
        {
            return Err(SyncError::JoinRejected);
        }
        contact.verify_selected(self.member_prefix(prefix)?)
    }

    pub fn sign_member_contact_native(
        &self,
        prefix: &PreparedV2JoinedPrefix,
        listener: (&[u8; PEER_BYTES], u16),
        (source, cut, ends): (
            &NativeChatSourceObservation,
            &mut NativeChatPreparedKeyCut,
            &mut NativeChatKeyEnds,
        ),
    ) -> Result<[u8; V2_MEMBER_CONTACT_BYTES], SyncError> {
        let (peer, port) = listener;
        if port == 0
            || !valid_peer_encoding(peer)
            || blake3::hash(peer).as_bytes() != self.local_peer_data().as_bytes()
            || !self
                .native_origin
                .as_ref()
                .is_some_and(|bound| bound.same_original(source))
        {
            return Err(SyncError::JoinRejected);
        }
        let original = self.observe_member_publication(prefix)?;
        let replay = self.member_prefix(prefix)?.replay_data();
        let signer = self
            .member_signer_data()
            .map_err(|_| SyncError::JoinRejected)?;
        let own = OwnerBodyId::from_bytes(*self.admission_id.as_bytes());
        if replay
            .slot(own)
            .is_none_or(|slot| slot.image.subject != signer)
        {
            return Err(SyncError::JoinRejected);
        }
        let mut raw = [0; V2_MEMBER_CONTACT_BYTES];
        let (preimage, signature) = raw.split_at_mut(PREIMAGE_BYTES);
        let mut out = &mut preimage[..];
        for field in [
            DOMAIN,
            replay.profile().id().as_bytes(),
            self.head.as_bytes(),
            own.as_bytes(),
            &self.epoch.to_be_bytes(),
            peer,
            &port.to_be_bytes(),
        ] {
            let (copy, tail) = out.split_at_mut(field.len());
            copy.copy_from_slice(field);
            out = tail;
        }
        if !original.is_current_cached() {
            return Err(SyncError::JoinRejected);
        }
        cut.enter(source, NativeChatKeyPurpose::MemberContact, ends)
            .map_err(|_| SyncError::JoinRejected)?;
        signature.copy_from_slice(
            &self
                .node
                .device
                .sign_retained_ed25519_bounded(&signer, preimage, PREIMAGE_BYTES)
                .map_err(|_| SyncError::JoinRejected)?,
        );
        if !original.is_current_cached() || cut.is_cancelled() {
            return Err(SyncError::JoinRejected);
        }
        Ok(raw)
    }
}
