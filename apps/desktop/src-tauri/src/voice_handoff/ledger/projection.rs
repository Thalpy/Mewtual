//! Non-creating outstanding-work discovery. Certified Plan-final subtrees stay available
//! through direct authenticated history, but do not pin a new automatic consumer. A slot
//! is a scan hint, never an event cursor: Pending -> attached changes the same RowId.
use super::*;

/// Fixed public metadata copied under the original Row guard. The actual Row remains paid
/// in WorkProjection until serialization finishes, including after unlink/End/collection.
pub(in crate::voice_handoff) struct WorkMetadata {
    pub row: RowId,
    pub purpose: Purpose,
    pub parent_selector: Option<PairSelector>,
    pub pending: PendingStatus,
    pub action: Option<(u64, u64, [u8; 32])>,
}
pub(in crate::voice_handoff) struct WorkProjection {
    pub next_slot: u8,
    pub metadata: Option<WorkMetadata>,
    // No Work, Phase, Input, grant upgrade, engine or copy loan is acquired by this pull.
    // Retain only this paid row (and its original parent), never Arc<Ledger>. A delayed
    // public reply must not keep the process index or unrelated rows alive after teardown.
    _row: Option<Arc<Row>>,
}

impl Ledger {
    /// Examine at most the existing32 slots and return one matching original row. Empty is
    /// merely this snapshot's result; an admitted native producer can publish afterward.
    /// Never infer channel/Root/SDK drainage, or discard unseen Pending, from an empty scan.
    pub(in crate::voice_handoff) fn project_work(
        self: &Arc<Self>,
        selected: &SelectedHandoffOwner,
        from: u8,
    ) -> Result<WorkProjection, VoiceError> {
        if usize::from(from) > ROWS {
            return Err(VoiceError::Bounds);
        }
        let index = self.index.lock().unwrap_or_else(|error| error.into_inner());
        let mut candidate = None;
        for row in index.rows.iter().skip(usize::from(from)).flatten() {
            // SDP is consumed explicitly by the original-key v8 round composer. Never
            // feed that distinct action family into the confirmation/v1 public pump.
            if matches!(row.key.purpose, Purpose::Sdp { .. })
                || !row.key.matches(selected, row.key.purpose)
            {
                continue;
            }
            // Exact native Plan-final issuance/ACK and full retirement certify that
            // this subtree has no outstanding automatic work. Match the ACTUAL parent
            // allocation, never a slot/selector. Direct original Control lookup/history
            // is retained, including recovery of a lost ACK observation. Empty metadata
            // remains insufficient evidence for any browser cleanup or advancement.
            let parent = row.key.parent.as_ref().unwrap_or(row);
            if Self::certified_retired_plan(&index, selected, parent)? {
                continue;
            }
            candidate = Some(row);
            break;
        }
        let Some(row) = candidate else {
            return Ok(WorkProjection {
                next_slot: ROWS as u8,
                metadata: None,
                _row: None,
            });
        };
        let state = row.state.lock().unwrap_or_else(|error| error.into_inner());
        let parent_selector = match row.key.purpose {
            Purpose::Derive(_) => None,
            Purpose::Confirmation { parent, .. } => {
                let original = row.key.parent.as_ref().ok_or(VoiceError::Unauthorized)?;
                let Purpose::Derive(selector) = original.key.purpose else {
                    return Err(VoiceError::Unauthorized);
                };
                if original.id != parent {
                    return Err(VoiceError::Unauthorized);
                }
                Some(selector)
            }
            Purpose::Sdp { .. } => return Err(VoiceError::Unauthorized),
        };
        let metadata = WorkMetadata {
            row: row.id,
            purpose: row.key.purpose,
            parent_selector,
            pending: state.status.pending,
            action: state.status.action,
        };
        Ok(WorkProjection {
            next_slot: (row.id.slot + 1) as u8,
            metadata: Some(metadata),
            _row: Some(row.clone()),
        })
    }
}

#[cfg(test)]
mod tests;
