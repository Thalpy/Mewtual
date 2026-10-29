//! Closed operation31/version12 on the separately keyed admitted physical Session.
//! These observations describe native custody, never Ready, role or SDK return.
use crate::voice_handoff::channel::BodyKind;
use catcoms_sync::voice_v2::VoiceError;

pub(in crate::voice_handoff) const BYTES: usize = 10;
pub(in crate::voice_handoff) const REPLY_BYTES: usize = 59;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::voice_handoff) enum Kind {
    SetupAck,
    Register,
    Observe,
    Cancel,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::voice_handoff) struct Request {
    pub kind: Kind,
    pub generation: u64,
}
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
    pub requested: u64,
    pub canonical: u64,
    pub cancelled: u64,
    pub incarnation: Option<[u8; 32]>,
}
pub(in crate::voice_handoff) fn wire_shape(
    kind: BodyKind,
    logical: [u8; 16],
    bytes: usize,
    session: [u8; 16],
) -> bool {
    kind == BodyKind::Control && logical == session && bytes == BYTES + 16
}
impl Request {
    pub(in crate::voice_handoff) fn decode(bytes: &[u8]) -> Result<Self, VoiceError> {
        if bytes.len() != BYTES || bytes[0] != 12 {
            return Err(VoiceError::Malformed);
        }
        let kind = match bytes[1] {
            0 => Kind::SetupAck,
            1 => Kind::Register,
            2 => Kind::Observe,
            3 => Kind::Cancel,
            _ => return Err(VoiceError::Malformed),
        };
        let generation = u64::from_be_bytes(bytes[2..].try_into().expect("fixed generation"));
        if (kind == Kind::SetupAck) != (generation == 0) {
            return Err(VoiceError::Bounds);
        }
        Ok(Self { kind, generation })
    }
}
impl Snapshot {
    pub(in crate::voice_handoff) fn encode(self) -> [u8; REPLY_BYTES] {
        let mut bytes = [0; REPLY_BYTES];
        bytes[0] = 12;
        bytes[1] = self.state as u8;
        bytes[2..10].copy_from_slice(&self.requested.to_be_bytes());
        bytes[10..18].copy_from_slice(&self.canonical.to_be_bytes());
        bytes[18..26].copy_from_slice(&self.cancelled.to_be_bytes());
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
    fn closed_admitted_v12_preserves_zero_identity_and_exact_ordinals() {
        for generation in [1, 1 << 53, u64::MAX] {
            for (tag, kind) in [(1, Kind::Register), (2, Kind::Observe), (3, Kind::Cancel)] {
                let mut bytes = [12, tag, 0, 0, 0, 0, 0, 0, 0, 0];
                bytes[2..].copy_from_slice(&generation.to_be_bytes());
                assert_eq!(Request::decode(&bytes), Ok(Request { kind, generation }));
                for version in [0, 10, 11, 13, 255] {
                    bytes[0] = version;
                    assert!(Request::decode(&bytes).is_err());
                }
            }
        }
        assert_eq!(
            Request::decode(&[12, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            Ok(Request {
                kind: Kind::SetupAck,
                generation: 0
            })
        );
        for tag in [1, 2, 3, 4, 255] {
            assert!(Request::decode(&[12, tag, 0, 0, 0, 0, 0, 0, 0, 0]).is_err());
        }
        for n in 0..BYTES {
            assert!(Request::decode(&[12; BYTES][..n]).is_err());
        }
        assert!(Request::decode(&[12; BYTES + 1]).is_err());
        let bytes = Snapshot {
            state: State::Reserved,
            requested: 1,
            canonical: 1,
            cancelled: 0,
            incarnation: Some([0; 32]),
        }
        .encode();
        assert_eq!(bytes[26], 1);
        assert_eq!(&bytes[27..], &[0; 32]);
    }
    #[test]
    fn admitted_v12_requests_match_the_shared_browser_vectors() {
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("admitted-admission-vectors.json")).unwrap();
        for vector in vectors["requests"].as_array().unwrap() {
            let kind = match vector["kind"].as_str().unwrap() {
                "setup-ack" => Kind::SetupAck,
                "register" => Kind::Register,
                "observe" => Kind::Observe,
                "cancel" => Kind::Cancel,
                _ => panic!("closed vector kind"),
            };
            assert_eq!(
                Request::decode(&hex::decode(vector["body"].as_str().unwrap()).unwrap()),
                Ok(Request {
                    kind,
                    generation: vector["generation"].as_str().unwrap().parse().unwrap()
                })
            );
        }
    }
}
