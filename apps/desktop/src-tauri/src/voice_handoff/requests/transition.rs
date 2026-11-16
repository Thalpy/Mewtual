//! Separate bounded transition grammar. All fields are lookup hints, not a native Claim.
//! Authenticate the original Session/AEAD and resolve the exact parent before using it.
use super::{Action, Cursor, PairSelector, VoiceError};
use crate::voice_handoff::ledger::RowId;

pub(in crate::voice_handoff) const CLAIM_BYTES: usize = 186;
pub(in crate::voice_handoff) const RELEASE_BYTES: usize = CLAIM_BYTES + 32;

pub(in crate::voice_handoff) enum Kind {
    Claim,
    Release([u8; 32]),
}
pub(in crate::voice_handoff) struct Request {
    pub selector: PairSelector,
    pub action: Action,
    pub handshake: [u8; 32],
    pub kind: Kind,
}
impl Request {
    /// Version3 never falls back to parent1/child2. A parent RowId is mandatory in the
    /// authenticated header; no child purpose, time, Finish evidence or bool is accepted.
    pub(in crate::voice_handoff) fn decode(
        logical: [u8; 16],
        bytes: &[u8],
    ) -> Result<Self, VoiceError> {
        RowId::from_logical(logical)?;
        let mut cursor = Cursor(bytes);
        if cursor.take::<1>()? != [3] {
            return Err(VoiceError::Malformed);
        }
        let selector = PairSelector {
            peer: catcoms_app::DeviceId::from_bytes(cursor.take()?),
            join: cursor.take()?,
            key_ref: cursor.take()?,
            epoch: u64::from_be_bytes(cursor.take()?),
        };
        let action = cursor.action()?;
        let handshake = cursor.take()?;
        let kind = match cursor.take::<1>()? {
            [0] => Kind::Claim,
            [1] => Kind::Release(cursor.take()?),
            _ => return Err(VoiceError::Malformed),
        };
        if !cursor.0.is_empty() {
            return Err(VoiceError::Malformed);
        }
        Ok(Self {
            selector,
            action,
            handshake,
            kind,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transition_request_vectors_pin_u64_parent_direction_and_closed_grammar() {
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("transition/vectors.json")).unwrap();
        for case in vectors["requests"].as_array().unwrap() {
            let logical = hex::decode(case["parentRow"].as_str().unwrap())
                .unwrap()
                .try_into()
                .unwrap();
            let bytes = hex::decode(case["body"].as_str().unwrap()).unwrap();
            let decoded = Request::decode(logical, &bytes).unwrap();
            assert_eq!(decoded.selector.epoch.to_string(), case["purpose"]["epoch"]);
            assert_eq!(
                decoded.action.serial.to_string(),
                case["identity"]["action"]
            );
            assert_eq!(
                decoded.action.version.to_string(),
                case["identity"]["version"]
            );
            assert_eq!(
                hex::encode(decoded.action.input_tag),
                case["identity"]["inputTag"]
            );
            assert_eq!(hex::encode(decoded.handshake), case["handshake"]);
            match decoded.kind {
                Kind::Claim => {
                    assert_eq!(case["request"]["kind"], "claim");
                    assert_eq!(bytes.len(), CLAIM_BYTES);
                }
                Kind::Release(receipt) => {
                    assert_eq!(case["request"]["kind"], "release");
                    assert_eq!(hex::encode(receipt), case["request"]["receipt"]);
                    assert_eq!(bytes.len(), RELEASE_BYTES);
                }
            }
            for end in 0..bytes.len() {
                assert!(Request::decode(logical, &bytes[..end]).is_err());
            }
            let mut changed = bytes.clone();
            changed.push(0);
            assert!(Request::decode(logical, &changed).is_err());
            for version in [0, 1, 2, 4, 255] {
                let mut changed = bytes.clone();
                changed[0] = version;
                assert!(Request::decode(logical, &changed).is_err());
            }
            let mut changed = bytes.clone();
            changed[185] = 2;
            assert!(Request::decode(logical, &changed).is_err());
            for range in [105..113, 113..121] {
                let mut changed = bytes.clone();
                changed[range].fill(0);
                assert!(Request::decode(logical, &changed).is_err());
            }
            assert!(Request::decode([0; 16], &bytes).is_err());
        }
    }
    #[test]
    fn transition_header_admission_refuses_impossible_class_length_and_parent_before_counter_debit()
    {
        use crate::voice_handoff::channel::{BodyKind, Operation};
        let mut parent = [0; 16];
        parent[15] = 1;
        for plain in 0..=256 {
            assert_eq!(
                super::super::wire_shape(
                    Operation::ChainTransition,
                    BodyKind::Control,
                    parent,
                    plain + 16,
                    [0; 16]
                ),
                plain == CLAIM_BYTES || plain == RELEASE_BYTES
            );
            assert!(!super::super::wire_shape(
                Operation::ChainTransition,
                BodyKind::Data,
                parent,
                plain + 16,
                [0; 16]
            ));
            assert!(!super::super::wire_shape(
                Operation::ChainTransition,
                BodyKind::Control,
                [0; 16],
                plain + 16,
                [0; 16]
            ));
        }
    }
}
