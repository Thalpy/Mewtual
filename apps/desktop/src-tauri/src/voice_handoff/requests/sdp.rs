//! Closed original-KeyClaim SDP control grammar. The authenticated Header's full parent
//! RowId identifies the existing native selector; the renderer cannot supply a new epoch,
//! peer or pair descriptor to redirect the retained keys. This parser grants no effect.
use super::{Action, Cursor};
use crate::voice_handoff::ledger::RowId;
use catcoms_sync::voice_v2::VoiceError;

pub(in crate::voice_handoff) const PREPARE_BYTES: usize = 122;
pub(in crate::voice_handoff) const RELEASE_BYTES: usize = 154;
pub(in crate::voice_handoff) enum Kind {
    Prepare,
    Release([u8; 32]),
}
pub(in crate::voice_handoff) struct Request {
    pub action: Action,
    pub handshake: [u8; 32],
    pub key: [u8; 32],
    pub sequence: u64,
    pub kind: Kind,
}
impl Request {
    pub(in crate::voice_handoff) fn decode(
        logical: [u8; 16],
        bytes: &[u8],
    ) -> Result<Self, VoiceError> {
        RowId::from_logical(logical)?;
        let mut cursor = Cursor(bytes);
        if cursor.take::<1>()? != [5] {
            return Err(VoiceError::Malformed);
        }
        let action = cursor.action()?;
        let handshake = cursor.take()?;
        let key = cursor.take()?;
        let sequence = u64::from_be_bytes(cursor.take()?);
        if sequence == 0 {
            return Err(VoiceError::Malformed);
        }
        let kind = match cursor.take::<1>()? {
            [0] => Kind::Prepare,
            [1] => Kind::Release(cursor.take()?),
            _ => return Err(VoiceError::Malformed),
        };
        if !cursor.0.is_empty() {
            return Err(VoiceError::Malformed);
        }
        Ok(Self {
            action,
            handshake,
            key,
            sequence,
            kind,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn body(release: bool) -> Vec<u8> {
        let mut bytes = vec![5];
        bytes.extend_from_slice(&9_007_199_254_740_993u64.to_be_bytes());
        bytes.extend_from_slice(&u64::MAX.to_be_bytes());
        bytes.extend_from_slice(&[17; 32]);
        bytes.extend_from_slice(&[18; 32]);
        bytes.extend_from_slice(&[19; 32]);
        bytes.extend_from_slice(&u64::MAX.to_be_bytes());
        bytes.push(u8::from(release));
        if release {
            bytes.extend_from_slice(&[20; 32]);
        }
        bytes
    }
    #[test]
    fn sdp_control_keeps_full_original_identities_and_u64_without_selector_fallback() {
        let mut logical = [0; 16];
        logical[15] = 1;
        for release in [false, true] {
            let bytes = body(release);
            assert_eq!(
                bytes.len(),
                if release {
                    RELEASE_BYTES
                } else {
                    PREPARE_BYTES
                }
            );
            let request = Request::decode(logical, &bytes).unwrap();
            assert_eq!(request.action.serial, 9_007_199_254_740_993);
            assert_eq!(request.action.version, u64::MAX);
            assert_eq!(request.action.input_tag, [17; 32]);
            assert_eq!(request.handshake, [18; 32]);
            assert_eq!(request.key, [19; 32]);
            assert_eq!(request.sequence, u64::MAX);
            match request.kind {
                Kind::Prepare => assert!(!release),
                Kind::Release(proof) => {
                    assert!(release);
                    assert_eq!(proof, [20; 32]);
                }
            }
            for end in 0..bytes.len() {
                assert!(Request::decode(logical, &bytes[..end]).is_err());
            }
            let mut changed = bytes.clone();
            changed.push(0);
            assert!(Request::decode(logical, &changed).is_err());
            for version in [0, 1, 2, 3, 4, 6, 255] {
                let mut changed = bytes.clone();
                changed[0] = version;
                assert!(Request::decode(logical, &changed).is_err());
            }
            for range in [1..9, 9..17, 113..121] {
                let mut changed = bytes.clone();
                changed[range].fill(0);
                assert!(Request::decode(logical, &changed).is_err());
            }
            for direction in [2, 3, 255] {
                let mut changed = bytes.clone();
                changed[121] = direction;
                assert!(Request::decode(logical, &changed).is_err());
            }
        }
        assert!(Request::decode([0; 16], &body(false)).is_err());
    }
    #[test]
    fn sdp_header_refuses_impossible_class_length_or_full_parent_before_body_admission() {
        use crate::voice_handoff::channel::{BodyKind, Operation};
        let mut id = [0; 16];
        id[15] = 1;
        for bytes in [PREPARE_BYTES, RELEASE_BYTES] {
            assert!(super::super::wire_shape(
                Operation::SdpPhase,
                BodyKind::Control,
                id,
                bytes + 16,
                [9; 16]
            ));
            assert!(!super::super::wire_shape(
                Operation::SdpPhase,
                BodyKind::Data,
                id,
                bytes + 16,
                [9; 16]
            ));
            assert!(!super::super::wire_shape(
                Operation::SdpPhase,
                BodyKind::Control,
                [0; 16],
                bytes + 16,
                [9; 16]
            ));
            for length in [bytes + 15, bytes + 17] {
                assert!(!super::super::wire_shape(
                    Operation::SdpPhase,
                    BodyKind::Control,
                    id,
                    length,
                    [9; 16]
                ));
            }
        }
    }

    #[test]
    fn native_sdp_storage_requests_match_browser_vectors() {
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("sdp-vectors.json")).unwrap();
        let id = hex::decode(vectors["parentRow"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        for vector in vectors["requests"].as_array().unwrap() {
            let bytes = hex::decode(vector["body"].as_str().unwrap()).unwrap();
            let request = Request::decode(id, &bytes).unwrap();
            assert_eq!(
                request.action.serial.to_string(),
                vectors["identity"]["action"]
            );
            assert_eq!(
                request.action.version.to_string(),
                vectors["identity"]["version"]
            );
            assert_eq!(
                hex::encode(request.action.input_tag),
                vectors["identity"]["inputTag"]
            );
            assert_eq!(hex::encode(request.handshake), vectors["handshake"]);
            assert_eq!(hex::encode(request.key), vectors["key"]);
            assert_eq!(request.sequence.to_string(), vectors["sequence"]);
            match request.kind {
                Kind::Prepare => assert_eq!(vector["name"], "prepare"),
                Kind::Release(proof) => {
                    assert_eq!(vector["name"], "release");
                    assert_eq!(hex::encode(proof), vectors["proof"]);
                }
            }
        }
    }
}
