//! Shared first-new-tenure checks for owner takeover of an already frozen source. This does
//! not reopen a gate or choose a seed: Studio/Registry still validate their actual full closure,
//! persist the exact decision, and use whole-source recovery before checkpoint adoption.

use super::*;

/// The inherited checkpoint is the INSTALLED opening, never an uninstalled remote target or
/// an old owner's journal. A same-tenure call is permitted only to resume the exact previously
/// journaled first-new-tenure decision; it cannot generate a replacement for a pending seal.
#[allow(clippy::too_many_arguments)]
pub(crate) fn frozen_owner_inheritance(
    document: &LogicalDocument,
    epoch: u64,
    opening: Option<&Receipt>,
    head: Option<&Receipt>,
    previous: Option<&Receipt>,
    group: &ServerGroup,
    tenure: u64,
) -> Result<InheritedCheckpoint, ReplError> {
    if document.server_id != group.group_id() || tenure > group.epoch() {
        return Err(ReplError::EpochScope);
    }
    let head = head.ok_or(ReplError::ReceiptConflict)?;
    for receipt in [opening, Some(head), previous].into_iter().flatten() {
        // Rust records can bypass the wire decoder. Scope and key checks precede allocation.
        if &receipt.document != document || receipt.owner_public_key.len() != 32 {
            return Err(ReplError::EpochScope);
        }
        Receipt::decode(&receipt.encode())?;
        receipt.restore_verified_from_vault()?;
    }
    let inherited = match opening {
        None if epoch == 0 => InheritedCheckpoint::EpochZero,
        Some(opening)
            if opening.closed_epoch.checked_add(1) == Some(epoch)
                && opening.tenure_start_group_epoch < tenure =>
        {
            InheritedCheckpoint::Checkpoint {
                epoch,
                close_record_hash: opening.close_record_hash,
                seed_change_hash: opening.seed_change_hash,
            }
        }
        _ => return Err(ReplError::ReceiptConflict),
    };
    if let Some(previous) = previous {
        if previous.tenure_start_group_epoch > tenure {
            return Err(ReplError::ReceiptConflict);
        }
        if previous.tenure_start_group_epoch == tenure {
            previous.verify_current_owner(group, tenure)?;
            if previous.closed_epoch != epoch || previous.inherited != inherited {
                return Err(ReplError::ReceiptConflict);
            }
        }
    }
    // Before the journal write the held seal belongs to an older tenure. After the write/seal
    // crash boundary it may be exactly that first current-tenure decision, never another one.
    if head.tenure_start_group_epoch >= tenure
        && !(head.tenure_start_group_epoch == tenure && previous == Some(head))
    {
        return Err(ReplError::ReceiptConflict);
    }
    Ok(inherited)
}
