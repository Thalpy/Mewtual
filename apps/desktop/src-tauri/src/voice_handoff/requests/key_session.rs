//! Distinct retention-only adoption grammar. Opcode10's finite Claim never changes.
//! Authenticate the original channel before resolving these lookup-only fields.
use super::{Action, Cursor, PairSelector, VoiceError};
use crate::voice_handoff::ledger::RowId;

pub(in crate::voice_handoff) const ADOPT_BYTES: usize = 218;
pub(in crate::voice_handoff) const RELEASE_BYTES: usize = 250;
pub(in crate::voice_handoff) enum Kind {
    Adopt,
    Release([u8; 32]),
}
pub(in crate::voice_handoff) struct Request {
    pub selector: PairSelector,
    pub action: Action,
    pub handshake: [u8; 32],
    pub claim: [u8; 32],
    pub kind: Kind,
}
impl Request {
    pub(in crate::voice_handoff) fn decode(
        logical: [u8; 16],
        bytes: &[u8],
    ) -> Result<Self, VoiceError> {
        RowId::from_logical(logical)?;
        let mut cursor = Cursor(bytes);
        if cursor.take::<1>()? != [4] {
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
        let direction = cursor.take::<1>()?;
        let claim = cursor.take()?;
        let kind = match direction {
            [0] => Kind::Adopt,
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
            claim,
            kind,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn established_keys_grammar_pins_vectors_u64_original_claim_and_closed_versions() {
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("key-session-vectors.json")).unwrap();
        let logical = hex::decode(vectors["parentRow"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        for vector in vectors["requests"].as_array().unwrap() {
            let body = hex::decode(vector["body"].as_str().unwrap()).unwrap();
            let decoded = Request::decode(logical, &body).unwrap();
            assert_eq!(decoded.selector.epoch, u64::MAX);
            assert_eq!(decoded.action.serial, 9_007_199_254_740_993);
            assert_eq!(decoded.action.version, u64::MAX);
            assert_eq!(decoded.claim, [0x66; 32]);
            assert_eq!(decoded.handshake, [0x55; 32]);
            match decoded.kind {
                Kind::Adopt => {
                    assert_eq!(vector["request"]["kind"], "adopt");
                    assert_eq!(body.len(), ADOPT_BYTES);
                }
                Kind::Release(proof) => {
                    assert_eq!(vector["request"]["kind"], "release");
                    assert_eq!(proof, [0x99; 32]);
                    assert_eq!(body.len(), RELEASE_BYTES);
                }
            }
            for end in 0..body.len() {
                assert!(Request::decode(logical, &body[..end]).is_err());
            }
            let mut trailing = body.clone();
            trailing.push(0);
            assert!(Request::decode(logical, &trailing).is_err());
            for version in [0, 1, 2, 3, 5, 255] {
                let mut changed = body.clone();
                changed[0] = version;
                assert!(Request::decode(logical, &changed).is_err());
            }
            for direction in [2, 3, 255] {
                let mut changed = body.clone();
                changed[185] = direction;
                assert!(Request::decode(logical, &changed).is_err());
            }
            for range in [105..113, 113..121] {
                let mut changed = body.clone();
                changed[range].fill(0);
                assert!(Request::decode(logical, &changed).is_err());
            }
            assert!(Request::decode([0; 16], &body).is_err());
        }
        for length in 0..=512 {
            assert_eq!(
                super::super::wire_shape(
                    super::super::Operation::KeySession,
                    crate::voice_handoff::channel::BodyKind::Control,
                    logical,
                    length + 16,
                    [0; 16]
                ),
                length == ADOPT_BYTES || length == RELEASE_BYTES
            );
            assert!(!super::super::wire_shape(
                super::super::Operation::KeySession,
                crate::voice_handoff::channel::BodyKind::Data,
                logical,
                length + 16,
                [0; 16]
            ));
        }
    }
}
