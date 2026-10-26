//! Closed media-custody grammar. Full forms authenticate original PC obligations even
//! when the first reply/media proof was lost. Compact forms select an EXISTING exact
//! native claim; knowing a deterministic proof cannot create it or admit a new peer.
//! Decoding allocates no descriptor/body and grants no native/Root/RTC authority.
use super::{pc, sdp, sdp_action, Action, Cursor};
use crate::voice_handoff::channel::BodyKind;
use crate::voice_handoff::ledger::RowId;
use catcoms_sync::voice_v2::VoiceError;

pub(in crate::voice_handoff) const ORIGINAL_BYTES: usize = sdp_action::BEGIN_BYTES + 1;
pub(in crate::voice_handoff) const EXISTING_BYTES: usize = 2 + 48 + 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::voice_handoff) enum OriginalKind {
    Admit,
    Resolve,
    Close,
    Release,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::voice_handoff) enum ExistingKind {
    Current,
    /// Move existing admitted custody after original source expiry; never admit a PC.
    Detach,
    Close,
    Release,
}
// Deliberately inline: this closed312B parse result is included in the physical Control
// inventory. Boxing would allocate before the media producer's separate task payment;
// parsing fixed public bindings must not add an unaccounted heap owner or failure point.
#[allow(clippy::large_enum_variant)]
pub(in crate::voice_handoff) enum Request {
    Original {
        original: sdp_action::Begin<'static>,
        kind: OriginalKind,
    },
    Existing {
        parent: RowId,
        action: Action,
        proof: [u8; 32],
        kind: ExistingKind,
    },
}
/// Necessary authenticated-header conditions only. Exact post-AEAD mode/version and
/// original native bindings remain mandatory; no generic Data/SDP body fallback exists.
pub(in crate::voice_handoff) fn wire_shape(
    kind: BodyKind,
    logical: [u8; 16],
    cipher_bytes: usize,
) -> bool {
    kind == BodyKind::Control
        && RowId::from_logical(logical).is_ok()
        && matches!(
            cipher_bytes.checked_sub(16),
            Some(ORIGINAL_BYTES | EXISTING_BYTES)
        )
}
impl Request {
    pub(in crate::voice_handoff) fn decode(
        logical: [u8; 16],
        bytes: &[u8],
    ) -> Result<Self, VoiceError> {
        if !wire_shape(
            BodyKind::Control,
            logical,
            bytes.len().checked_add(16).ok_or(VoiceError::Bounds)?,
        ) {
            return Err(VoiceError::Bounds);
        }
        let parent = RowId::from_logical(logical)?;
        let mut cursor = Cursor(bytes);
        if cursor.take::<1>()? != [9] {
            return Err(VoiceError::Malformed);
        }
        let mode = cursor.take::<1>()?[0];
        let action = cursor.action()?;
        let result = if bytes.len() == ORIGINAL_BYTES {
            let kind = match mode {
                0 => OriginalKind::Admit,
                1 => OriginalKind::Resolve,
                2 => OriginalKind::Close,
                3 => OriginalKind::Release,
                _ => return Err(VoiceError::Malformed),
            };
            let handshake = cursor.take()?;
            let key = cursor.take()?;
            let sequence = u64::from_be_bytes(cursor.take()?);
            let storage = cursor.take()?;
            let sdk_generation = u64::from_be_bytes(cursor.take()?);
            let sdk = cursor.take()?;
            let proof = cursor.take()?;
            if sequence == 0 || sdk_generation == 0 {
                return Err(VoiceError::Malformed);
            }
            Self::Original {
                original: sdp_action::Begin {
                    parent,
                    pc: pc::Request {
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
                        kind: pc::Kind::Admit,
                    },
                    proof,
                    plaintext: None,
                },
                kind,
            }
        } else {
            let kind = match mode {
                4 => ExistingKind::Current,
                5 => ExistingKind::Close,
                6 => ExistingKind::Release,
                7 => ExistingKind::Detach,
                _ => return Err(VoiceError::Malformed),
            };
            Self::Existing {
                parent,
                action,
                proof: cursor.take()?,
                kind,
            }
        };
        if !cursor.0.is_empty() {
            return Err(VoiceError::Malformed);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn vectors() -> serde_json::Value {
        serde_json::from_str(include_str!("media-vectors.json")).unwrap()
    }
    #[test]
    fn media_shared_vectors_pin_full_original_u64_and_compact_mode_separation() {
        let vectors = vectors();
        let parent: [u8; 16] = hex::decode(vectors["original"]["parentRow"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        for vector in vectors["requests"].as_array().unwrap() {
            let body = hex::decode(vector["body"].as_str().unwrap()).unwrap();
            let request = Request::decode(parent, &body).unwrap();
            match request {
                Request::Original { original, kind } => {
                    assert_eq!(body.len(), ORIGINAL_BYTES);
                    assert_eq!(
                        original.pc.base.action.serial.to_string(),
                        vectors["original"]["identity"]["action"]
                    );
                    assert_eq!(
                        original.pc.base.action.version.to_string(),
                        vectors["original"]["identity"]["version"]
                    );
                    assert_eq!(
                        hex::encode(original.pc.base.action.input_tag),
                        vectors["original"]["identity"]["inputTag"]
                    );
                    assert_eq!(hex::encode(original.proof), vectors["original"]["pcProof"]);
                    assert_eq!(
                        original.pc.sdk_generation.to_string(),
                        vectors["original"]["sdkGeneration"]
                    );
                    assert_eq!(
                        format!("{kind:?}").to_lowercase(),
                        vector["name"]
                            .as_str()
                            .unwrap()
                            .strip_prefix("original-")
                            .unwrap()
                    );
                    assert!(original.plaintext.is_none());
                }
                Request::Existing {
                    action,
                    proof,
                    kind,
                    ..
                } => {
                    assert_eq!(body.len(), EXISTING_BYTES);
                    assert_eq!(
                        action.serial.to_string(),
                        vectors["original"]["identity"]["action"]
                    );
                    assert_eq!(hex::encode(proof), vectors["mediaProof"]);
                    assert_eq!(
                        format!("{kind:?}").to_lowercase(),
                        vector["name"]
                            .as_str()
                            .unwrap()
                            .strip_prefix("existing-")
                            .unwrap()
                    );
                }
            }
            assert!(wire_shape(BodyKind::Control, parent, body.len() + 16));
            assert!(!wire_shape(BodyKind::Data, parent, body.len() + 16));
            assert!(!wire_shape(BodyKind::Sdp, parent, body.len() + 16));
            assert!(!wire_shape(BodyKind::Control, [0; 16], body.len() + 16));
            for length in 0..body.len() {
                assert!(Request::decode(parent, &body[..length]).is_err());
            }
            let mut trailing = body.clone();
            trailing.push(0);
            assert!(Request::decode(parent, &trailing).is_err());
            for version in [0, 8, 10, 255] {
                let mut changed = body.clone();
                changed[0] = version;
                assert!(Request::decode(parent, &changed).is_err());
            }
            for mode in 0..=255 {
                let mut changed = body.clone();
                changed[1] = mode;
                let permitted = if body.len() == ORIGINAL_BYTES {
                    mode <= 3
                } else {
                    (4..=7).contains(&mode)
                };
                assert_eq!(Request::decode(parent, &changed).is_ok(), permitted);
            }
        }
    }
}
