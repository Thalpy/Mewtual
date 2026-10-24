//! First PC-intent control grammar. All identities are original authenticated native
//! obligations; callers cannot select Ready, a role, a PC generation, or extend a deadline.
use super::{sdp, Cursor};
use crate::voice_handoff::ledger::RowId;
use catcoms_sync::voice_v2::VoiceError;
pub(in crate::voice_handoff) const BYTES: usize = 194;
#[derive(Clone, Copy)]
pub(in crate::voice_handoff) enum Kind {
    Admit,
    Close,
}
pub(in crate::voice_handoff) struct Request {
    pub base: sdp::Request,
    pub storage: [u8; 32],
    pub sdk_generation: u64,
    pub sdk: [u8; 32],
    pub kind: Kind,
}
impl Request {
    pub(in crate::voice_handoff) fn decode(
        logical: [u8; 16],
        bytes: &[u8],
    ) -> Result<Self, VoiceError> {
        RowId::from_logical(logical)?;
        let mut cursor = Cursor(bytes);
        if cursor.take::<1>()? != [7] {
            return Err(VoiceError::Malformed);
        }
        let action = cursor.action()?;
        let handshake = cursor.take()?;
        let key = cursor.take()?;
        let sequence = u64::from_be_bytes(cursor.take()?);
        let storage = cursor.take()?;
        let sdk_generation = u64::from_be_bytes(cursor.take()?);
        let sdk = cursor.take()?;
        let kind = match cursor.take::<1>()? {
            [0] => Kind::Admit,
            [1] => Kind::Close,
            _ => return Err(VoiceError::Malformed),
        };
        if sequence == 0 || sdk_generation == 0 || !cursor.0.is_empty() {
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
            sdk_generation,
            sdk,
            kind,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pc_native_browser_vectors_pin_exact_original_obligations_and_header_class() {
        use crate::voice_handoff::channel::{BodyKind, Operation};
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("pc-vectors.json")).unwrap();
        let row: [u8; 16] = hex::decode(vectors["parentRow"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        for vector in vectors["requests"].as_array().unwrap() {
            let bytes = hex::decode(vector["body"].as_str().unwrap()).unwrap();
            let request = Request::decode(row, &bytes).unwrap();
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
            assert_eq!(request.sdk_generation.to_string(), vectors["sdkGeneration"]);
            assert_eq!(hex::encode(request.sdk), vectors["sdkProof"]);
            match request.kind {
                Kind::Admit => assert_eq!(vector["name"], "admit"),
                Kind::Close => assert_eq!(vector["name"], "close"),
            }
            assert!(super::super::wire_shape(
                Operation::PcIntent,
                BodyKind::Control,
                row,
                bytes.len() + 16,
                [1; 16]
            ));
            assert!(!super::super::wire_shape(
                Operation::PcIntent,
                BodyKind::Data,
                row,
                bytes.len() + 16,
                [1; 16]
            ));
            assert!(!super::super::wire_shape(
                Operation::PcIntent,
                BodyKind::Control,
                [0; 16],
                bytes.len() + 16,
                [1; 16]
            ));
            for delta in [15, 17] {
                assert!(!super::super::wire_shape(
                    Operation::PcIntent,
                    BodyKind::Control,
                    row,
                    bytes.len() + delta,
                    [1; 16]
                ));
            }
        }
    }
    #[test]
    fn pc_request_is_closed_and_never_accepts_renderer_ready_generation_or_time() {
        let mut row = [0; 16];
        row[15] = 1;
        let mut bytes = vec![7];
        bytes.extend_from_slice(&9_007_199_254_740_993u64.to_be_bytes());
        bytes.extend_from_slice(&u64::MAX.to_be_bytes());
        bytes.extend_from_slice(&[17; 32]);
        bytes.extend_from_slice(&[18; 32]);
        bytes.extend_from_slice(&[19; 32]);
        bytes.extend_from_slice(&1u64.to_be_bytes());
        bytes.extend_from_slice(&[20; 32]);
        bytes.extend_from_slice(&u64::MAX.to_be_bytes());
        bytes.extend_from_slice(&[21; 32]);
        bytes.push(0);
        assert_eq!(bytes.len(), BYTES);
        for kind in 0..=1 {
            bytes[193] = kind;
            let decoded = Request::decode(row, &bytes).unwrap();
            assert_eq!(decoded.base.action.serial, 9_007_199_254_740_993);
            assert_eq!(decoded.sdk_generation, u64::MAX);
            for end in 0..BYTES {
                assert!(Request::decode(row, &bytes[..end]).is_err());
            }
            let mut longer = bytes.clone();
            longer.push(0);
            assert!(Request::decode(row, &longer).is_err());
            for range in [1..9, 9..17, 113..121, 153..161] {
                let mut zero = bytes.clone();
                zero[range].fill(0);
                assert!(Request::decode(row, &zero).is_err());
            }
        }
        bytes[193] = 2;
        assert!(Request::decode(row, &bytes).is_err());
    }
}
