//! Distinct SDK custody, never PC/RTC permission. Full parent identity is in the authenticated
//! Header. The existing storage receipt and all original key/session bindings are required.
use super::{sdp, Action, Cursor};
use crate::voice_handoff::ledger::RowId;
use catcoms_sync::voice_v2::VoiceError;
pub(in crate::voice_handoff) const ADMIT_BYTES: usize = 154;
pub(in crate::voice_handoff) const RELEASE_BYTES: usize = 194;
pub(in crate::voice_handoff) enum Kind {
    Admit,
    Release { generation: u64, proof: [u8; 32] },
}
pub(in crate::voice_handoff) struct Request {
    pub base: sdp::Request,
    pub storage: [u8; 32],
    pub kind: Kind,
}
impl Request {
    pub(in crate::voice_handoff) fn decode(
        logical: [u8; 16],
        bytes: &[u8],
    ) -> Result<Self, VoiceError> {
        RowId::from_logical(logical)?;
        let mut cursor = Cursor(bytes);
        if cursor.take::<1>()? != [6] {
            return Err(VoiceError::Malformed);
        }
        let action: Action = cursor.action()?;
        let handshake = cursor.take()?;
        let key = cursor.take()?;
        let sequence = u64::from_be_bytes(cursor.take()?);
        if sequence == 0 {
            return Err(VoiceError::Malformed);
        }
        let storage = cursor.take()?;
        let kind = match cursor.take::<1>()? {
            [0] => Kind::Admit,
            [1] => {
                let generation = u64::from_be_bytes(cursor.take()?);
                if generation == 0 {
                    return Err(VoiceError::Malformed);
                }
                Kind::Release {
                    generation,
                    proof: cursor.take()?,
                }
            }
            _ => return Err(VoiceError::Malformed),
        };
        if !cursor.0.is_empty() {
            return Err(VoiceError::Malformed);
        }
        Ok(Self {
            base: sdp::Request {
                action,
                handshake,
                key,
                sequence,
                kind: sdp::Kind::Prepare,
            },
            storage,
            kind,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sdk_custody_vectors_preserve_exact_original_storage_and_u64_generation() {
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("sdk-vectors.json")).unwrap();
        let id = hex::decode(vectors["parentRow"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        for vector in vectors["requests"].as_array().unwrap() {
            let bytes = hex::decode(vector["body"].as_str().unwrap()).unwrap();
            let request = Request::decode(id, &bytes).unwrap();
            assert_eq!(
                request.base.action.serial.to_string(),
                vectors["identity"]["action"]
            );
            assert_eq!(
                request.base.action.version.to_string(),
                vectors["identity"]["version"]
            );
            assert_eq!(
                hex::encode(request.base.action.input_tag),
                vectors["identity"]["inputTag"]
            );
            assert_eq!(hex::encode(request.base.handshake), vectors["handshake"]);
            assert_eq!(hex::encode(request.base.key), vectors["key"]);
            assert_eq!(request.base.sequence.to_string(), vectors["sequence"]);
            assert_eq!(hex::encode(request.storage), vectors["storage"]);
            match request.kind {
                Kind::Admit => assert_eq!(vector["name"], "admit"),
                Kind::Release { generation, proof } => {
                    assert_eq!(vector["name"], "release");
                    assert_eq!(generation.to_string(), vectors["generation"]);
                    assert_eq!(hex::encode(proof), vectors["proof"]);
                }
            }
            for end in 0..bytes.len() {
                assert!(Request::decode(id, &bytes[..end]).is_err());
            }
            let mut changed = bytes.clone();
            changed.push(0);
            assert!(Request::decode(id, &changed).is_err());
            for offset in [0, 153] {
                let mut changed = bytes.clone();
                changed[offset] = 255;
                assert!(Request::decode(id, &changed).is_err());
            }
            for range in [1..9, 9..17, 113..121] {
                let mut changed = bytes.clone();
                changed[range].fill(0);
                assert!(Request::decode(id, &changed).is_err());
            }
            if bytes.len() == RELEASE_BYTES {
                let mut changed = bytes.clone();
                changed[154..162].fill(0);
                assert!(Request::decode(id, &changed).is_err());
            }
        }
    }
    #[test]
    fn sdk_header_admission_never_borrows_data_capacity_or_a_missing_parent() {
        use crate::voice_handoff::channel::{BodyKind, Operation};
        let mut id = [0; 16];
        id[15] = 1;
        for length in [ADMIT_BYTES, RELEASE_BYTES] {
            assert!(super::super::wire_shape(
                Operation::SdkCustody,
                BodyKind::Control,
                id,
                length + 16,
                [9; 16]
            ));
            assert!(!super::super::wire_shape(
                Operation::SdkCustody,
                BodyKind::Data,
                id,
                length + 16,
                [9; 16]
            ));
            assert!(!super::super::wire_shape(
                Operation::SdkCustody,
                BodyKind::Control,
                [0; 16],
                length + 16,
                [9; 16]
            ));
            for delta in [15, 17] {
                assert!(!super::super::wire_shape(
                    Operation::SdkCustody,
                    BodyKind::Control,
                    id,
                    length + delta,
                    [9; 16]
                ));
            }
        }
    }
}
