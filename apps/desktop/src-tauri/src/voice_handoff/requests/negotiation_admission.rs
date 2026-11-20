//! Session-level admission-mode selection, before setup ACK can start pilot work.
//! This local Control grammar creates no Root registration, availability or Ready.
//! In particular, its acknowledgement must never be used as a call/RTC permission.
use crate::voice_handoff::channel::BodyKind;
use catcoms_sync::voice_v2::VoiceError;

pub(in crate::voice_handoff) const BYTES: usize = 10;
pub(in crate::voice_handoff) const REPLY_BYTES: usize = 18;

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

/// The actual physical Session, not a ledger RowId, is the logical identity. This
/// necessary header check runs after MAC authentication and before counter debit.
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
        if bytes.len() != BYTES || bytes[0] != 10 {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admission_mode_grammar_has_one_explicit_version_and_exact_u64_ordinals() {
        for generation in [1, 1 << 53, u64::MAX] {
            for (mode, kind) in [(0, Kind::Register), (1, Kind::Observe), (2, Kind::Cancel)] {
                let mut bytes = [10, mode, 0, 0, 0, 0, 0, 0, 0, 0];
                bytes[2..].copy_from_slice(&generation.to_be_bytes());
                assert_eq!(Request::decode(&bytes), Ok(Request { kind, generation }));
                for version in [0, 1, 9, 11, 255] {
                    bytes[0] = version;
                    assert_eq!(Request::decode(&bytes), Err(VoiceError::Malformed));
                }
            }
        }
        for bytes in [
            [10, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            [10, 2, 0, 0, 0, 0, 0, 0, 0, 0],
        ] {
            assert_eq!(Request::decode(&bytes), Err(VoiceError::Bounds));
        }
        assert_eq!(
            Request::decode(&[10; BYTES - 1]),
            Err(VoiceError::Malformed)
        );
        assert_eq!(
            Request::decode(&[10; BYTES + 1]),
            Err(VoiceError::Malformed)
        );
        assert_eq!(Request::decode(&[10; BYTES]), Err(VoiceError::Malformed));
    }

    #[test]
    fn admission_mode_header_requires_the_exact_session_control_and_length() {
        let session = [19; 16];
        assert!(wire_shape(BodyKind::Control, session, BYTES + 16, session));
        for kind in [BodyKind::Data, BodyKind::Sdp] {
            assert!(!wire_shape(kind, session, BYTES + 16, session));
        }
        assert!(!wire_shape(
            BodyKind::Control,
            [20; 16],
            BYTES + 16,
            session
        ));
        for length in [0, BYTES, BYTES + 15, BYTES + 17, usize::MAX] {
            assert!(!wire_shape(BodyKind::Control, session, length, session));
        }
    }
}
