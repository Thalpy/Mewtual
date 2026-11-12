//! Warm original-aware first-B verifier DATA. Exact comparisons bind all original
//! opaque SDK/Core outputs; this is not a cold-adoption or parsed-WAL permission.
use super::*;
pub(super) fn verify_original_b_data(
    cell: &PreparedV2TextAcceptedCreate,
    key: &[u8; 32],
    wal: &TextWorkWalFrameData<'_>,
) -> Result<(), AppError> {
    let p = &cell.parent.head;
    let expected = cell.candidate.as_ref().ok_or_else(refused)?.frame_data();
    let plan = RetirementPlanData::decode_data(cell.plan.as_ref().ok_or_else(refused)?)?;
    if wal.operation != TextWorkWalOperationData::Accept
        || wal.context != p.context.id_data()?
        || wal.transaction != cell.signature.record.transaction
        || wal.public_parent != cell.signature.record.public_parent
        || wal.parent_frame != cell.parent.head_frame.as_slice()
        || wal.install_parent != TextHeadIdData::of_data(&cell.parent.head_frame)
        || wal.candidate_frame != expected
        || wal.candidate != TextHeadIdData::of_data(expected)
        || plan.context_data() != wal.context
        || plan.transaction_data() != wal.transaction
        || plan.selected_data() != wal.candidate
    {
        return Err(refused());
    }
    let TextWorkWalCleanupData::Final {
        plan_commit,
        plan: actual,
        deletion_frame,
    } = wal.cleanup
    else {
        return Err(refused());
    };
    if plan_commit != plan.id_data()?
        || actual != cell.plan.as_ref().ok_or_else(refused)?.as_slice()
        || deletion_frame != cell.deletion.as_ref().ok_or_else(refused)?.frame_data()
    {
        return Err(refused());
    }
    let original = cell
        .signature
        .intent
        .unsigned
        .unsigned_data(&cell.signature.intent.retained)?;
    let actual_candidate = cell
        .signature
        .intent
        .unsigned
        .borrow_accepted_candidate_data(
            &cell.signature.intent.retained,
            &cell.observe_original(),
        )?;
    if actual_candidate.header_data() != &cell.parent.header
        || actual_candidate.image_data() != cell.parent.image.as_slice()
        || original.body.id() != cell.parent.header.spec().body
        || cell.parent.header.spec().parent != wal.public_parent
        || !cell.parent.header.verify()
        || cell.parent.header.signature()
            != cell
                .signature
                .intent
                .unsigned
                .existing_raw_signature_data()?
    {
        return Err(refused());
    }
    let plain = open_control_data(key, TextControlKindData::Head.id_data(expected), expected)?;
    let candidate = TextHeadData::decode_data(&plain)?;
    candidate.require_successor_data(p, wal.install_parent)?;
    let (bytes, count) = cell.artifact_totals_data()?;
    if candidate.pending != TextPendingData::None
        || candidate.queue_front != 0
        || candidate.queue_next != 1
        || candidate.live_bytes != p.live_bytes.checked_add(bytes).ok_or_else(refused)?
        || candidate.live_objects != p.live_objects.checked_add(count).ok_or_else(refused)?
        || bytes > REMAINING_AFTER_S_BYTES
        || count > REMAINING_AFTER_S_OBJECTS
    {
        return Err(refused());
    }
    let expected_artifacts = cell.artifact_frames_data()?;
    let mut actual = wal.artifacts.entries_data();
    for e in &expected_artifacts {
        let a = actual.next().ok_or_else(refused)??;
        if a.role_data() != e.role_data()
            || a.id_data() != e.id_data()
            || a.frame_data() != e.frame_data()
        {
            return Err(refused());
        }
    }
    if actual.next().is_some() {
        return Err(refused());
    }
    // The bounded retirement plan comes only from this actual completed Directory
    // path's observed length, the actual A token and exact original I/S. Public
    // histories, shared siblings and synthetic max-size entries cannot enter it.
    let old = RetirementCarryData::decode_data(&cell.signature.carry)?;
    let mut roots = old.roots_data().to_vec();
    roots.push(wal.install_parent);
    roots.sort_unstable();
    if old.roots_data().len() != 3
        || !old.entries_data().is_empty()
        || plan.carry_data().roots_data() != roots
    {
        return Err(refused());
    }
    let entries = plan.carry_data().entries_data();
    let expected_entries = [
        RetirementEntryData {
            kind: RetirementKindData::AcceptedIndex,
            id: *cell.parent.directory_root.page.as_bytes(),
            frame_bytes: u32::try_from(cell.parent.directory_frame_bytes).map_err(|_| refused())?,
        },
        RetirementEntryData {
            kind: RetirementKindData::TokenIndex,
            id: *cell.parent.token_root.page.as_bytes(),
            frame_bytes: u32::try_from(cell.parent.token_frame.len()).map_err(|_| refused())?,
        },
        RetirementEntryData {
            kind: RetirementKindData::Intent,
            id: *cell.signature.record.intent.as_bytes(),
            frame_bytes: u32::try_from(cell.signature.parent.intent_frame.len())
                .map_err(|_| refused())?,
        },
        RetirementEntryData {
            kind: RetirementKindData::Signature,
            id: *cell
                .signature
                .signature
                .as_ref()
                .ok_or_else(refused)?
                .id_data()
                .as_bytes(),
            frame_bytes: 405,
        },
    ];
    if entries != expected_entries {
        return Err(refused());
    }
    Ok(())
}

#[cfg(all(test, windows))]
mod tests;
