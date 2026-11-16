//! Fixed private action-request grammar for the native/browser handoff. Decoding is NOT
//! authentication or authorization: the dispatcher must first authenticate/decrypt the
//! original Session, then resolve the SAME header RowId, actual selection and full purpose.
//! This module allocates no body/descriptor and cannot run a factory or borrow an engine.
use super::channel::Operation;
use super::custody::CloseMode;
use catcoms_app::DeviceId;
use catcoms_sync::voice_v2::crypto_actions::{CryptoOutcome, CryptoResultReceipt};
use catcoms_sync::voice_v2::presence::PairSelector;
use catcoms_sync::voice_v2::VoiceError;
use sha2::{Digest, Sha256};
pub(super) mod key_session;
pub(super) mod media;
pub(super) mod pc;
pub(super) mod sdk;
pub(super) mod sdp;
pub(super) mod sdp_action;
pub(super) mod transition;

/// Header-only necessary conditions for the closed v1 parent / v2 confirmation grammars.
/// Call only AFTER authenticating the immutable Header, BEFORE allocating Data or debiting
/// either directional counter. These are not sufficient conditions: AEAD and the exact
/// versioned decoder still validate every field and original native purpose afterward.
///
/// Prefix sizes are the v1 selector (105 bytes), or selector + parent + purpose tag with
/// Start(None), Incoming(role), Respond(row), or Start(predecessor). The encrypted version
/// cannot be known here, so lengths are the union, never a decoder fallback. Keep this table
/// pinned to both shared native/browser vector sets when introducing a new grammar.
pub(super) fn wire_shape(
    operation: Operation,
    kind: super::channel::BodyKind,
    logical: [u8; 16],
    cipher_bytes: usize,
    session: [u8; 16],
) -> bool {
    use super::channel::BodyKind;
    if operation == Operation::Media {
        return media::wire_shape(kind, logical, cipher_bytes);
    }
    if operation.is_sdp() {
        return sdp_action::wire_shape(operation, kind, logical, cipher_bytes);
    }
    const PREFIXES: [usize; 5] = [105, 122, 123, 138, 154];
    let Some(plain_bytes) = cipher_bytes.checked_sub(16) else {
        return false;
    };
    if operation == Operation::Status {
        return kind == BodyKind::Control && logical == session && plain_bytes == 49;
    }
    if operation == Operation::BeginDerive {
        return kind == BodyKind::Control
            && logical == [0; 16]
            && PREFIXES.iter().any(|prefix| plain_bytes == prefix + 1);
    }
    if super::ledger::RowId::from_logical(logical).is_err()
        || kind
            != if operation == Operation::Result {
                BodyKind::Data
            } else {
                BodyKind::Control
            }
    {
        return false;
    }
    if operation == Operation::PcIntent {
        return plain_bytes == pc::BYTES;
    }
    if operation == Operation::SdkCustody {
        return plain_bytes == sdk::ADMIT_BYTES || plain_bytes == sdk::RELEASE_BYTES;
    }
    if operation == Operation::SdpPhase {
        return plain_bytes == sdp::PREPARE_BYTES || plain_bytes == sdp::RELEASE_BYTES;
    }
    if operation == Operation::KeySession {
        return plain_bytes == key_session::ADOPT_BYTES
            || plain_bytes == key_session::RELEASE_BYTES;
    }
    if operation == Operation::ChainTransition {
        return plain_bytes == transition::CLAIM_BYTES || plain_bytes == transition::RELEASE_BYTES;
    }
    if operation == Operation::FinalAck {
        return plain_bytes == 105 + 32; // Only the original v1 parent owns final ACK.
    }
    PREFIXES.iter().any(|prefix| {
        let Some(suffix) = plain_bytes.checked_sub(*prefix) else {
            return false;
        };
        match operation {
            Operation::Input => suffix == 0,
            Operation::Admit => suffix == 48 + 4,
            Operation::Result => (48 + 32 + 2..=48 + 32 + 2 + 528).contains(&suffix),
            Operation::ReceiptAck => suffix == 48 + 32,
            Operation::Settle => suffix == 48,
            Operation::ClosePhase => suffix == 1,
            _ => false,
        }
    })
}

/// Exact identity of the original native action, in addition to full purpose and RowId.
/// A copied identity is a lookup hint only, never a fresh engine or publication permit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Action {
    pub serial: u64,
    pub version: u64,
    pub input_tag: [u8; 32],
}
pub(super) enum Kind<'a> {
    /// False recovers historical state without admitting a factory. True may additionally
    /// request currently authorized retry of an idle Pending row, on its ORIGINAL serial.
    Begin {
        retry: bool,
    },
    Input,
    Admit {
        action: Action,
        engine_bytes: u32,
    },
    Result {
        action: Action,
        tag: [u8; 32],
        output: &'a [u8],
    },
    ReceiptAck {
        action: Action,
        digest: [u8; 32],
    },
    Settle {
        action: Action,
    },
    ClosePhase {
        mode: CloseMode,
    },
    /// Separate whole-plan intent after ordinary action Returns. This cannot relabel an
    /// earlier Phase closure, and requires the original native UI-commit admission.
    ClosePlan,
    FinalAck {
        proof: [u8; 32],
    },
}
pub(super) struct Request<'a> {
    pub selector: PairSelector,
    pub kind: Kind<'a>,
}

impl<'a> Request<'a> {
    /// Versioned canonical plaintext, within the already authenticated frame's exact length.
    /// No trailing bytes, alternate booleans or zero action identities are accepted. Epoch
    /// may be zero; all integers retain their full u64/u32 value without a JSON number hop.
    pub(super) fn decode(operation: Operation, bytes: &'a [u8]) -> Result<Self, VoiceError> {
        let mut cursor = Cursor(bytes);
        if cursor.take::<1>()? != [1] {
            return Err(VoiceError::Malformed);
        }
        let selector = PairSelector {
            peer: DeviceId::from_bytes(cursor.take()?),
            join: cursor.take()?,
            key_ref: cursor.take()?,
            epoch: u64::from_be_bytes(cursor.take()?),
        };
        let kind = decode_kind(operation, cursor.0)?;
        Ok(Self { selector, kind })
    }
}
/// Shared closed operation suffix. The v2 child grammar owns its separate prefix/type;
/// reusing this borrowing parser never constructs a v1 body or enlarges v1 Request.
fn decode_kind(operation: Operation, bytes: &[u8]) -> Result<Kind<'_>, VoiceError> {
    let mut cursor = Cursor(bytes);
    let kind = match operation {
        Operation::BeginDerive => Kind::Begin {
            retry: match cursor.take::<1>()? {
                [0] => false,
                [1] => true,
                _ => return Err(VoiceError::Malformed),
            },
        },
        Operation::Input => Kind::Input,
        Operation::Admit => Kind::Admit {
            action: cursor.action()?,
            engine_bytes: u32::from_be_bytes(cursor.take()?),
        },
        Operation::Result => {
            let action = cursor.action()?;
            let tag = cursor.take()?;
            let length = usize::from(u16::from_be_bytes(cursor.take()?));
            if length > 528 || cursor.0.len() != length {
                return Err(VoiceError::Bounds);
            }
            let output = cursor.0;
            cursor.0 = &[];
            Kind::Result {
                action,
                tag,
                output,
            }
        }
        Operation::ReceiptAck => Kind::ReceiptAck {
            action: cursor.action()?,
            digest: cursor.take()?,
        },
        Operation::Settle => Kind::Settle {
            action: cursor.action()?,
        },
        Operation::ClosePhase => match cursor.take::<1>()? {
            [0] => Kind::ClosePhase {
                mode: CloseMode::Return,
            },
            [1] => Kind::ClosePhase {
                mode: CloseMode::Retire,
            },
            [2] => Kind::ClosePlan,
            _ => return Err(VoiceError::Malformed),
        },
        Operation::FinalAck => Kind::FinalAck {
            proof: cursor.take()?,
        },
        _ => return Err(VoiceError::Malformed),
    };
    if !cursor.0.is_empty() {
        return Err(VoiceError::Malformed);
    }
    Ok(kind)
}

pub(super) mod confirmation;

struct Cursor<'a>(&'a [u8]);
impl Cursor<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], VoiceError> {
        let prefix = self.0.get(..N).ok_or(VoiceError::Malformed)?;
        let bytes = prefix.try_into().expect("fixed checked prefix");
        self.0 = &self.0[N..];
        Ok(bytes)
    }
    fn action(&mut self) -> Result<Action, VoiceError> {
        let action = Action {
            serial: u64::from_be_bytes(self.take()?),
            version: u64::from_be_bytes(self.take()?),
            input_tag: self.take()?,
        };
        if action.serial == 0 || action.version == 0 {
            return Err(VoiceError::Malformed);
        }
        Ok(action)
    }
}

/// ACK identity follows Root's same_transition exactly. `retired` is an observation which
/// may change after End, not part of a past transition. A renderer supplies this digest only;
/// ACK must compare the ACTUAL retained native receipt and pass that original receipt to Root.
/// Canonical closed outcome/error tags are deliberately independent of Debug/Display strings.
pub(super) fn receipt_digest(receipt: &CryptoResultReceipt) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"Mewtual native voice action receipt v1\0");
    hash.update(receipt.action.to_be_bytes());
    hash.update(receipt.input_tag);
    hash.update(receipt.output_tag);
    match receipt.outcome {
        CryptoOutcome::Derived => hash.update([0]),
        CryptoOutcome::Sealed {
            message,
            hash: sealed_hash,
        } => {
            hash.update([1]);
            hash.update(message);
            hash.update(sealed_hash);
        }
        CryptoOutcome::Opened => hash.update([2]),
        CryptoOutcome::Rejected(error) => hash.update([3, error_code(error)]),
    }
    hash.finalize().into()
}
/// Closed private IPC tag table shared by request receipt digests and fixed replies.
pub(super) fn error_code(error: VoiceError) -> u8 {
    match error {
        VoiceError::Malformed => 0,
        VoiceError::Bounds => 1,
        VoiceError::Signature => 2,
        VoiceError::Scope => 3,
        VoiceError::Freshness => 4,
        VoiceError::Unauthorized => 5,
        VoiceError::Signing => 6,
        VoiceError::Capacity => 7,
        VoiceError::Replay => 8,
        VoiceError::Reservation => 9,
        VoiceError::RetryMismatch => 10,
        VoiceError::RateLimited => 11,
    }
}

#[cfg(test)]
mod tests;
