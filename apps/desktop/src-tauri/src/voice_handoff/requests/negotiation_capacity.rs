//! Exact physical-Session capacity grammar. This is local authenticated Control,
//! not a P2P voice message or permission to emit Ready, create a ledger row or use RTC.
use crate::voice_handoff::channel::BodyKind;
use catcoms_sync::voice_v2::VoiceError;

pub(in crate::voice_handoff) const BYTES: usize = 10;
pub(in crate::voice_handoff) const REPLY_BYTES: usize = 59;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::voice_handoff) enum Kind {
    Register,
    Observe,
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::voice_handoff) struct Request {
    pub kind: Kind,
    pub generation: u64,
}

/// Each value describes only the requested generation's native capacity custody.
/// In particular Returned/Superseded do not certify browser SDK drainage, task or
/// Actor payer refund, or positive permission for any other/current generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(in crate::voice_handoff) enum State {
    Unstarted = 0,
    Preparing = 1,
    Waiting = 2,
    Reserved = 3,
    Closing = 4,
    Returned = 5,
    Closed = 6,
    Uncertain = 7,
    Superseded = 8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::voice_handoff) struct Snapshot {
    pub state: State,
    pub requested_generation: u64,
    pub canonical_generation: u64,
    pub cancelled_through: u64,
    // Public Root-RNG identity only. Never expose private physical Session bytes.
    // Presence is explicit: an all-zero random public identity is still valid.
    pub incarnation: Option<[u8; 32]>,
}

/// Necessary authenticated-header check BEFORE counter admission or RowId parsing.
pub(in crate::voice_handoff) fn wire_shape(
    kind: BodyKind,
    logical: [u8; 16],
    cipher_bytes: usize,
    session: [u8; 16],
) -> bool {
    kind == BodyKind::Control && logical == session && cipher_bytes == BYTES + 16
}

impl Request {
    pub(in crate::voice_handoff) fn decode(bytes: &[u8]) -> Result<Self, VoiceError> {
        if bytes.len() != BYTES || bytes[0] != 11 {
            return Err(VoiceError::Malformed);
        }
        let kind = match bytes[1] {
            0 => Kind::Register,
            1 => Kind::Observe,
            2 => Kind::Cancel,
            _ => return Err(VoiceError::Malformed),
        };
        let generation = u64::from_be_bytes(bytes[2..].try_into().expect("fixed ordinal"));
        if generation == 0 {
            return Err(VoiceError::Bounds);
        }
        Ok(Self { kind, generation })
    }
}

impl Snapshot {
    pub(in crate::voice_handoff) fn encode(self) -> [u8; REPLY_BYTES] {
        let mut bytes = [0; REPLY_BYTES];
        bytes[0] = 11;
        bytes[1] = self.state as u8;
        bytes[2..10].copy_from_slice(&self.requested_generation.to_be_bytes());
        bytes[10..18].copy_from_slice(&self.canonical_generation.to_be_bytes());
        bytes[18..26].copy_from_slice(&self.cancelled_through.to_be_bytes());
        if let Some(incarnation) = self.incarnation {
            bytes[26] = 1;
            bytes[27..].copy_from_slice(&incarnation);
        }
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_and_mode_match_shared_browser_vectors() {
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("negotiation-capacity-vectors.json")).unwrap();
        for vector in vectors["requests"].as_array().unwrap() {
            let expected = match vector["kind"].as_str().unwrap() {
                "register" => Kind::Register,
                "observe" => Kind::Observe,
                "cancel" => Kind::Cancel,
                _ => panic!("closed fixture kind"),
            };
            assert_eq!(
                Request::decode(&hex::decode(vector["body"].as_str().unwrap()).unwrap()),
                Ok(Request {
                    kind: expected,
                    generation: vector["generation"].as_str().unwrap().parse().unwrap()
                })
            );
        }
        let states = [
            State::Unstarted,
            State::Preparing,
            State::Waiting,
            State::Reserved,
            State::Closing,
            State::Returned,
            State::Closed,
            State::Uncertain,
            State::Superseded,
        ];
        for (vector, state) in vectors["replies"].as_array().unwrap().iter().zip(states) {
            let snapshot = Snapshot {
                state,
                requested_generation: vector["generation"].as_str().unwrap().parse().unwrap(),
                canonical_generation: vector["canonical"].as_str().unwrap().parse().unwrap(),
                cancelled_through: vector["floor"].as_str().unwrap().parse().unwrap(),
                incarnation: vector["incarnation"]
                    .as_str()
                    .map(|value| hex::decode(value).unwrap().try_into().unwrap()),
            };
            assert_eq!(
                hex::encode(snapshot.encode()),
                vector["body"].as_str().unwrap()
            );
        }
        for (vector, kind) in vectors["modeRequests"].as_array().unwrap().iter().zip([
            super::super::negotiation_admission::Kind::Register,
            super::super::negotiation_admission::Kind::Observe,
            super::super::negotiation_admission::Kind::Cancel,
        ]) {
            assert_eq!(
                super::super::negotiation_admission::Request::decode(
                    &hex::decode(vector["body"].as_str().unwrap()).unwrap()
                )
                .unwrap()
                .kind,
                kind
            );
        }
    }

    #[test]
    fn capacity_grammar_refuses_mode_version_truncation_extra_fields_and_zero_before_mutation() {
        for generation in [1_u64, 1 << 53, u64::MAX] {
            for (mode, kind) in [(0, Kind::Register), (1, Kind::Observe), (2, Kind::Cancel)] {
                let mut bytes = [0; BYTES];
                bytes[0] = 11;
                bytes[1] = mode;
                bytes[2..].copy_from_slice(&generation.to_be_bytes());
                assert_eq!(Request::decode(&bytes), Ok(Request { kind, generation }));
                for version in [0, 1, 10, 12, 255] {
                    bytes[0] = version;
                    assert_eq!(Request::decode(&bytes), Err(VoiceError::Malformed));
                }
            }
        }
        assert_eq!(
            Request::decode(&[11, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            Err(VoiceError::Bounds)
        );
        assert_eq!(Request::decode(&[11; BYTES]), Err(VoiceError::Malformed));
        assert_eq!(
            Request::decode(&[11; BYTES - 1]),
            Err(VoiceError::Malformed)
        );
        assert_eq!(
            Request::decode(&[11; BYTES + 1]),
            Err(VoiceError::Malformed)
        );
    }

    #[test]
    fn capacity_header_is_exact_session_control_and_short_length_only() {
        let session = [4; 16];
        assert!(wire_shape(BodyKind::Control, session, BYTES + 16, session));
        for kind in [BodyKind::Data, BodyKind::Sdp] {
            assert!(!wire_shape(kind, session, BYTES + 16, session));
        }
        assert!(!wire_shape(BodyKind::Control, [5; 16], BYTES + 16, session));
        for length in [0, BYTES, BYTES + 15, BYTES + 17, usize::MAX] {
            assert!(!wire_shape(BodyKind::Control, session, length, session));
        }
    }

    #[test]
    fn capacity_snapshot_binds_request_and_canonical_ordinals_without_zero_nonce_sentinels() {
        let mut snapshot = Snapshot {
            state: State::Superseded,
            requested_generation: 1 << 53,
            canonical_generation: u64::MAX,
            cancelled_through: u64::MAX - 1,
            incarnation: Some([0; 32]),
        };
        let bytes = snapshot.encode();
        assert_eq!(&bytes[..2], &[11, 8]);
        assert_eq!(&bytes[2..10], &(1_u64 << 53).to_be_bytes());
        assert_eq!(&bytes[10..18], &u64::MAX.to_be_bytes());
        assert_eq!(&bytes[18..26], &(u64::MAX - 1).to_be_bytes());
        assert_eq!(bytes[26], 1);
        assert_eq!(&bytes[27..], &[0; 32]);
        snapshot.incarnation = None;
        assert_eq!(&snapshot.encode()[26..], &[0; 33]);
    }
}
