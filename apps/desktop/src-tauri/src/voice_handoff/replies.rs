//! Fixed private action history replies. This codec grants no authority and enables no
//! listener operation. Authenticate the original Session and resolve the complete native
//! purpose BEFORE encoding; the original prepaid Control owner must cover this stack/output.
use super::factory::Refused;
use super::ledger::{PendingStatus, RowId, Status};
use super::requests::{error_code, receipt_digest};
use catcoms_sync::voice_v2::crypto_actions::CryptoOutcome;
use catcoms_sync::voice_v2::VoiceError;

pub(super) const STATUS_BYTES: usize = 242;

/// One canonical fixed historical observation, never a fresh factory/input/engine permit.
/// The digest names the ACTUAL native receipt and excludes observational `retired`, exactly
/// like Root's same_transition. ACK must still resolve that original receipt through native
/// history; a browser digest or decoded receipt cannot manufacture a Root receipt.
pub(super) fn status(id: RowId, status: &Status) -> Result<[u8; STATUS_BYTES], VoiceError> {
    RowId::from_logical(id.logical())?;
    validate(status)?;
    let mut bytes = [0; STATUS_BYTES];
    bytes[0] = 1; // version; result kind0 = original history
    bytes[2..18].copy_from_slice(&id.logical());
    bytes[18] = match status.pending {
        PendingStatus::NotPending => 0,
        PendingStatus::Open => 1,
        PendingStatus::Closing => 2,
        PendingStatus::Closed {
            actor_handoff: false,
        } => 3,
        PendingStatus::Closed {
            actor_handoff: true,
        } => 4,
    };
    if let Some((serial, version, input)) = status.action {
        bytes[19] = 1;
        bytes[20..28].copy_from_slice(&serial.to_be_bytes());
        bytes[28..36].copy_from_slice(&version.to_be_bytes());
        bytes[36..68].copy_from_slice(&input);
    }
    bytes[68] = u8::from(status.engine.settled);
    if let Some(receipt) = &status.engine.receipt {
        bytes[69] = 1;
        bytes[70..78].copy_from_slice(&receipt.action.to_be_bytes());
        bytes[78..110].copy_from_slice(&receipt.input_tag);
        bytes[110..142].copy_from_slice(&receipt.output_tag);
        match receipt.outcome {
            CryptoOutcome::Derived => {} // tag0 and unused payload stay zero
            CryptoOutcome::Sealed { message, hash } => {
                bytes[142] = 1;
                bytes[143..175].copy_from_slice(&message);
                bytes[175..207].copy_from_slice(&hash);
            }
            CryptoOutcome::Opened => bytes[142] = 2,
            CryptoOutcome::Rejected(error) => {
                bytes[142] = 3;
                bytes[143] = error_code(error);
            }
        }
        bytes[207] = u8::from(receipt.retired);
        bytes[210..242].copy_from_slice(&receipt_digest(receipt));
    }
    bytes[208] = u8::from(status.acknowledged);
    bytes[209] = u8::from(status.drained); // LOCAL Phase only, never full Workflow proof
    Ok(bytes)
}

/// Closed/Stopped describe refusal only: neither means missing work or successful drainage.
/// No Display/Debug strings or unbounded diagnostic data enter this private wire grammar.
pub(super) fn refused(error: Refused) -> [u8; 3] {
    match error {
        Refused::Closed => [1, 1, 0],
        Refused::Stopped => [1, 2, 0],
        Refused::Protocol(error) => [1, 3, error_code(error)],
    }
}

fn validate(status: &Status) -> Result<(), VoiceError> {
    if status.pending != PendingStatus::NotPending {
        if status.action.is_some()
            || status.engine.settled
            || status.engine.receipt.is_some()
            || status.acknowledged
            || status.drained
        {
            return Err(VoiceError::Malformed);
        }
        return Ok(());
    }
    let (serial, version, input) = status.action.ok_or(VoiceError::Malformed)?;
    if serial == 0 || version == 0 {
        return Err(VoiceError::Malformed);
    }
    if let Some(receipt) = &status.engine.receipt {
        if receipt.action != serial || receipt.input_tag != input {
            return Err(VoiceError::Malformed);
        }
        if status.drained && (!status.acknowledged || !receipt.retired) {
            return Err(VoiceError::Malformed);
        }
    } else if status.acknowledged {
        return Err(VoiceError::Malformed);
    }
    if status.drained && !status.engine.settled {
        return Err(VoiceError::Malformed);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
