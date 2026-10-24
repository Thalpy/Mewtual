//! Separate child grammar. These fields are lookup hints, NEVER native parent,
//! Action, incoming-token or engine authority. Authenticate the original Session first;
//! resolve the actual parent by its full selector/RowId, then the actual child's SAME parent
//! allocation, purpose and header RowId. The dispatcher never falls back to the parent grammar.
use super::{decode_kind, Cursor, Kind, Operation, PairSelector, VoiceError};
use crate::voice_handoff::channel::BodyKind;
use crate::voice_handoff::custody::CloseMode;
use crate::voice_handoff::ledger::{ConfirmationPurpose, RowId};

/// No allocation or owned output copy. Its result slice borrows the original admitted body.
/// Keep this type separate: growing v1 Request would enlarge all existing paid adapters.
pub(in crate::voice_handoff) struct Request<'a> {
    pub selector: PairSelector,
    pub parent: RowId,
    pub purpose: ConfirmationPurpose,
    pub row: Option<RowId>,
    pub kind: Kind<'a>,
}
impl<'a> Request<'a> {
    pub(in crate::voice_handoff) fn decode(
        operation: Operation,
        body_kind: BodyKind,
        logical: [u8; 16],
        bytes: &'a [u8],
    ) -> Result<Self, VoiceError> {
        if matches!(
            operation,
            Operation::Status
                | Operation::FinalAck
                | Operation::ChainTransition
                | Operation::KeySession
        ) || body_kind
            != if operation == Operation::Result {
                BodyKind::Data
            } else {
                BodyKind::Control
            }
        {
            return Err(VoiceError::Unauthorized);
        }
        let mut cursor = Cursor(bytes);
        if cursor.take::<1>()? != [2] {
            return Err(VoiceError::Malformed); // No fallback to the parent/v1 grammar.
        }
        let selector = PairSelector {
            peer: catcoms_app::DeviceId::from_bytes(cursor.take()?),
            join: cursor.take()?,
            key_ref: cursor.take()?,
            epoch: u64::from_be_bytes(cursor.take()?),
        };
        let parent = RowId::from_logical(cursor.take()?)?;
        let purpose = match cursor.take::<1>()? {
            [0] => ConfirmationPurpose::Start(None),
            [1] => ConfirmationPurpose::Start(Some(cursor.take()?)),
            [2] => ConfirmationPurpose::Respond(RowId::from_logical(cursor.take()?)?),
            [3] => match cursor.take::<1>()? {
                [role @ 1..=9] => ConfirmationPurpose::Incoming(role),
                _ => return Err(VoiceError::Malformed),
            },
            _ => return Err(VoiceError::Malformed),
        };
        let row = if operation == Operation::BeginDerive {
            if logical != [0; 16] {
                return Err(VoiceError::Unauthorized);
            }
            None
        } else {
            let row = RowId::from_logical(logical)?;
            if row == parent {
                return Err(VoiceError::Unauthorized);
            }
            Some(row)
        };
        let kind = decode_kind(operation, cursor.0)?;
        if matches!(
            kind,
            Kind::ClosePhase {
                mode: CloseMode::Retire
            } | Kind::ClosePlan
                | Kind::FinalAck { .. }
        ) {
            // A child can return only its own Phase. Only the original parent can retire
            // or acknowledge the whole plan; metadata cannot transfer that obligation.
            return Err(VoiceError::Unauthorized);
        }
        Ok(Self {
            selector,
            parent,
            purpose,
            row,
            kind,
        })
    }
}

#[cfg(test)]
mod tests;
