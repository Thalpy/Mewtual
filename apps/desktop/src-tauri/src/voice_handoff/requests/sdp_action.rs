//! Closed version8 SDP grammar, separate from confirmation's528-byte result and2048-byte
//! Data class. Decoding borrows the admitted allocation; it grants no Root/SDK authority.
//! The authenticated header selects the original parent or action row before large intake.
use super::{pc, Action, Cursor};
use crate::voice_handoff::channel::{BodyKind, Operation};
use crate::voice_handoff::ledger::RowId;
use catcoms_sync::voice_v2::chain::sdp_handoff::NativeRowPurpose;
use catcoms_sync::voice_v2::{VoiceError, MAX_SDP_PLAINTEXT};

pub(in crate::voice_handoff) const BEGIN_BYTES: usize = pc::BYTES - 1 + 32;
const ROW_BYTES: usize = 1 + 16 + 1;
const ACTION_BYTES: usize = ROW_BYTES + 48;
const RESULT_BYTES: usize = ACTION_BYTES + 32 + 2;
/// Largest ciphertext in this distinct class: native parent bindings plus one complete
/// plaintext SDP and its u16 length. Never use this ceiling for generic Data or Control.
pub(in crate::voice_handoff) const MAX_CIPHER_BYTES: usize =
    BEGIN_BYTES + 2 + MAX_SDP_PLAINTEXT + 16;
/// Input's separate ceiling includes bounded AAD/nonce/descriptor fields. The actual
/// input encoder must inventory those fields and its copies before this response is used.
pub(in crate::voice_handoff) const INPUT_CIPHER_BYTES: usize = MAX_SDP_PLAINTEXT + 1024 + 16;

pub(in crate::voice_handoff) struct Begin<'a> {
    pub parent: RowId,
    pub pc: pc::Request,
    pub proof: [u8; 32],
    /// None is incoming Open discovery. Seal's role and Offer/Answer are native-chosen.
    pub plaintext: Option<&'a [u8]>,
}
pub(in crate::voice_handoff) enum Kind<'a> {
    Input(Action),
    Admit {
        action: Action,
        engine_bytes: u32,
    },
    Sealed {
        action: Action,
        tag: [u8; 32],
        bytes: &'a [u8],
    },
    Opened {
        action: Action,
        tag: [u8; 32],
        bytes: &'a [u8],
    },
    AuthenticationFailed(Action),
    Ack {
        action: Action,
        digest: [u8; 32],
    },
    Settle(Action),
    Return,
    History,
    Delivery {
        action: Action,
        applied: bool,
    },
}
pub(in crate::voice_handoff) struct Existing<'a> {
    pub row: RowId,
    pub parent: RowId,
    pub purpose: NativeRowPurpose,
    pub kind: Kind<'a>,
}
pub(in crate::voice_handoff) enum Request<'a> {
    Begin(Begin<'a>),
    /// Lost Begin may hide both native RowId and Action. Resolve/Return through its
    /// original parent+PC+purpose after End without reopening storage or sending SDP.
    Resolve {
        original: Begin<'a>,
        purpose: NativeRowPurpose,
        close: bool,
    },
    Existing(Existing<'a>),
}

/// Necessary fixed header checks, called after MAC and before any body/counter admission.
/// Full encrypted parent/PC/SDK/action/purpose bindings remain mandatory after AEAD. Begin
/// uses its original parent RowId; every later operation uses its distinct native action row.
pub(in crate::voice_handoff) fn wire_shape(
    operation: Operation,
    kind: BodyKind,
    logical: [u8; 16],
    cipher_bytes: usize,
) -> bool {
    if RowId::from_logical(logical).is_err() {
        return false;
    }
    let Some(bytes) = cipher_bytes.checked_sub(16) else {
        return false;
    };
    match (operation, kind) {
        (Operation::SdpSealBegin, BodyKind::Sdp) => {
            (BEGIN_BYTES + 3..=BEGIN_BYTES + 2 + MAX_SDP_PLAINTEXT).contains(&bytes)
        }
        (Operation::SdpOpenBegin, BodyKind::Control) => bytes == BEGIN_BYTES,
        (Operation::SdpResolve, BodyKind::Control) => bytes == BEGIN_BYTES + 2,
        (
            Operation::SdpInput | Operation::SdpAuthenticationFailed | Operation::SdpSettle,
            BodyKind::Control,
        ) => bytes == ACTION_BYTES,
        (Operation::SdpEngineAdmit, BodyKind::Control) => bytes == ACTION_BYTES + 4,
        (Operation::SdpSealedResult, BodyKind::Sdp) => {
            (RESULT_BYTES + 17..=RESULT_BYTES + MAX_SDP_PLAINTEXT + 16).contains(&bytes)
        }
        (Operation::SdpOpenedResult, BodyKind::Sdp) => {
            (RESULT_BYTES..=RESULT_BYTES + MAX_SDP_PLAINTEXT).contains(&bytes)
        }
        (Operation::SdpReceiptAck, BodyKind::Control) => bytes == ACTION_BYTES + 32,
        (Operation::SdpReturn | Operation::SdpHistory, BodyKind::Control) => bytes == ROW_BYTES,
        (Operation::SdpDelivery, BodyKind::Control) => bytes == ACTION_BYTES + 1,
        _ => false,
    }
}
impl<'a> Request<'a> {
    pub(in crate::voice_handoff) fn decode(
        operation: Operation,
        kind: BodyKind,
        logical: [u8; 16],
        bytes: &'a [u8],
    ) -> Result<Self, VoiceError> {
        if !wire_shape(
            operation,
            kind,
            logical,
            bytes.len().checked_add(16).ok_or(VoiceError::Bounds)?,
        ) {
            return Err(VoiceError::Bounds);
        }
        let row = RowId::from_logical(logical)?;
        let mut cursor = Cursor(bytes);
        if cursor.take::<1>()? != [8] {
            return Err(VoiceError::Malformed);
        }
        if matches!(
            operation,
            Operation::SdpSealBegin | Operation::SdpOpenBegin | Operation::SdpResolve
        ) {
            let action = cursor.action()?;
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
            let plaintext = if operation == Operation::SdpSealBegin {
                Some(payload(&mut cursor, 1, MAX_SDP_PLAINTEXT)?)
            } else {
                None
            };
            let original = Begin {
                parent: row,
                pc: pc::Request {
                    base: super::sdp::Request {
                        action,
                        handshake,
                        key,
                        sequence,
                        kind: super::sdp::Kind::Prepare,
                    },
                    storage,
                    sdk_generation,
                    sdk,
                    kind: pc::Kind::Admit,
                },
                proof,
                plaintext,
            };
            let request = if operation == Operation::SdpResolve {
                let purpose = purpose(&mut cursor)?;
                let close = match cursor.take::<1>()? {
                    [0] => false,
                    [1] => true,
                    _ => return Err(VoiceError::Malformed),
                };
                Self::Resolve {
                    original,
                    purpose,
                    close,
                }
            } else {
                Self::Begin(original)
            };
            if !cursor.0.is_empty() {
                return Err(VoiceError::Malformed);
            }
            return Ok(request);
        }
        let parent = RowId::from_logical(cursor.take()?)?;
        if row == parent {
            return Err(VoiceError::Unauthorized);
        }
        let purpose = purpose(&mut cursor)?;
        let kind = match operation {
            Operation::SdpInput => Kind::Input(cursor.action()?),
            Operation::SdpEngineAdmit => Kind::Admit {
                action: cursor.action()?,
                engine_bytes: u32::from_be_bytes(cursor.take()?),
            },
            Operation::SdpSealedResult if purpose == NativeRowPurpose::Seal => Kind::Sealed {
                action: cursor.action()?,
                tag: cursor.take()?,
                bytes: payload(&mut cursor, 17, MAX_SDP_PLAINTEXT + 16)?,
            },
            Operation::SdpOpenedResult if purpose == NativeRowPurpose::Open => Kind::Opened {
                action: cursor.action()?,
                tag: cursor.take()?,
                // A valid tag-only Open has zero plaintext. Preserve that exact
                // successful SDK result; it is not AuthenticationFailed or SDP validity.
                bytes: payload(&mut cursor, 0, MAX_SDP_PLAINTEXT)?,
            },
            Operation::SdpAuthenticationFailed if purpose == NativeRowPurpose::Open => {
                Kind::AuthenticationFailed(cursor.action()?)
            }
            Operation::SdpReceiptAck => Kind::Ack {
                action: cursor.action()?,
                digest: cursor.take()?,
            },
            Operation::SdpSettle => Kind::Settle(cursor.action()?),
            Operation::SdpReturn => Kind::Return,
            Operation::SdpHistory => Kind::History,
            Operation::SdpDelivery if purpose == NativeRowPurpose::Open => Kind::Delivery {
                action: cursor.action()?,
                applied: match cursor.take::<1>()? {
                    [0] => false,
                    [1] => true,
                    _ => return Err(VoiceError::Malformed),
                },
            },
            _ => return Err(VoiceError::Unauthorized),
        };
        if !cursor.0.is_empty() {
            return Err(VoiceError::Malformed);
        }
        Ok(Self::Existing(Existing {
            row,
            parent,
            purpose,
            kind,
        }))
    }
}
fn purpose(cursor: &mut Cursor<'_>) -> Result<NativeRowPurpose, VoiceError> {
    match cursor.take::<1>()? {
        [0] => Ok(NativeRowPurpose::Seal),
        [1] => Ok(NativeRowPurpose::Open),
        _ => Err(VoiceError::Malformed),
    }
}
fn payload<'a>(
    cursor: &mut Cursor<'a>,
    minimum: usize,
    maximum: usize,
) -> Result<&'a [u8], VoiceError> {
    let length = usize::from(u16::from_be_bytes(cursor.take()?));
    if !(minimum..=maximum).contains(&length) || length != cursor.0.len() {
        return Err(VoiceError::Bounds);
    }
    let bytes = cursor.0;
    cursor.0 = &[];
    Ok(bytes)
}

#[cfg(test)]
mod tests;
