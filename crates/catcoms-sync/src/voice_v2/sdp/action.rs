//! Fixed native SDP action identity, shared by actual crypto/packet/Source/delivery owners.
//! It retains only independently paid PC/Ready provenance, never Exchange, Phase, a native
//! row, plaintext or an SDK result. Two direction records fit the existing PC metadata.
use super::*;
use crate::voice_v2::{EncryptedHeader, EncryptedKind, OfferId};
use futures::FutureExt;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::voice_v2) enum Direction {
    Seal,
    Open,
}
pub(in crate::voice_v2) struct ActionBinding {
    // The kind follows the immutable round/role/direction. Retain a fixed body hash
    // instead, so changed plaintext cannot retry an action whose input has compacted.
    body_tag: [u8; 32],
    offer: OfferId,
    direction: Direction,
    // Last: this independently paid original observation covers the complete fixed
    // record through destruction, including after every live phase/intent owner drops.
    intent: Arc<InitialSdpIntent>,
}
redacted_debug!(ActionBinding, Direction);
impl ActionBinding {
    /// Call only after the original affine direction issue is held and before publication.
    /// Equal header/round labels never establish authority: the creating LocalJoin must also
    /// resolve its SAME stored intent and actual unpublished outgoing or retained token.
    #[allow(clippy::too_many_arguments)] // Native issue/header provenance and clock samples are separate gates.
    pub(in crate::voice_v2) fn prepare(
        issue: &mut crate::voice_v2::negotiation::ActionIssue,
        intent: Arc<InitialSdpIntent>,
        header: &EncryptedHeader,
        message: Nonce,
        direction: Direction,
        body_tag: [u8; 32],
        wall: u64,
        mono: u64,
    ) -> Result<Arc<Self>, VoiceError> {
        let original = intent.original.binding();
        let sealing = direction == Direction::Seal;
        let offer = header.offer.ok_or(VoiceError::Unauthorized)?;
        let local_offer = intent.local_is_low() == sealing;
        let expected = if local_offer {
            EncryptedKind::OfferRound(intent.round())
        } else {
            EncryptedKind::AnswerRound(intent.round())
        };
        let (source_join, source_key, target, target_join, target_key) = if sealing {
            (
                original.local_join,
                original.local_key,
                original.peer,
                original.peer_join,
                original.peer_key,
            )
        } else {
            (
                original.peer_join,
                original.peer_key,
                original.local,
                original.local_join,
                original.local_key,
            )
        };
        let low = if original.low {
            original.local
        } else {
            original.peer
        };
        if !issue.covers_sdp_intent(&intent, sealing)
            || !intent.current_at(wall, mono)
            || header.kind != expected
            || header.sequence == 0
            || header.handshake != original.handshake
            || header.source_join != source_join
            || header.source_key_ref != source_key
            || header.target != target
            || header.target_join != target_join
            || header.target_key_ref != target_key
            || header.low_to_high != (sealing == original.low)
            || offer.issuer != low
            || (local_offer && offer.message != message)
        {
            return Err(VoiceError::Unauthorized);
        }
        issue.claim_sdp_binding()?;
        Ok(Arc::new(Self {
            intent,
            body_tag,
            offer,
            direction,
        }))
    }
    pub(in crate::voice_v2) fn current_at(&self, wall: u64, mono: u64) -> bool {
        self.intent.current_at(wall, mono)
    }
    pub(in crate::voice_v2) fn deadline(&self) -> u64 {
        self.intent.ready.deadline()
    }
    pub(in crate::voice_v2) fn offer(&self) -> OfferId {
        self.offer
    }
    pub(in crate::voice_v2) fn handshake(&self) -> Nonce {
        self.intent.handshake()
    }
    pub(in crate::voice_v2) fn peer(&self) -> DeviceId {
        self.intent.original.binding().peer
    }
    pub(in crate::voice_v2) fn local(&self) -> DeviceId {
        self.intent.original.binding().local
    }
    pub(in crate::voice_v2) fn matches_answer(
        &self,
        handshake: Nonce,
        kind: EncryptedKind,
        offer: Option<OfferId>,
    ) -> bool {
        !self.is_seal()
            && !self.intent.local_is_low()
            && handshake == self.handshake()
            && kind == EncryptedKind::AnswerRound(self.intent.round())
            && offer == Some(self.offer)
    }
    pub(in crate::voice_v2) async fn closed(&self) {
        let original = self.intent.original.binding();
        futures::future::select_all(vec![
            self.intent.effect.closed().boxed(),
            original.call.wait_revoked().boxed(),
            original.pair.wait_revoked().boxed(),
        ])
        .await;
    }
    pub(in crate::voice_v2) fn is_seal(&self) -> bool {
        self.direction == Direction::Seal
    }
    pub(in crate::voice_v2) fn matches_plaintext(&self, bytes: &[u8]) -> bool {
        self.is_seal() && self.body_tag == plaintext_tag(bytes)
    }
    #[cfg(test)]
    pub(in crate::voice_v2) fn body_tag_for_test(&self) -> [u8; 32] {
        self.body_tag
    }
    pub(in crate::voice_v2) fn matches_plaintext_commitment(&self, tag: &[u8; 32]) -> bool {
        self.is_seal() && self.body_tag == *tag
    }
    pub(in crate::voice_v2) fn belongs_to_intent(&self, intent: &Arc<InitialSdpIntent>) -> bool {
        Arc::ptr_eq(&self.intent, intent)
    }
    pub(in crate::voice_v2) fn matches_packet(
        &self,
        packet: &crate::voice_v2::packets::InboundPacket,
    ) -> bool {
        !self.is_seal() && self.body_tag == packet.object_hash()
    }
}

pub(in crate::voice_v2) fn plaintext_tag(bytes: &[u8]) -> [u8; 32] {
    let mut hash = blake3::Hasher::new();
    hash.update(b"mewtual/voice/native-sdp-plaintext/v1\0");
    hash.update(&(bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
    *hash.finalize().as_bytes()
}
