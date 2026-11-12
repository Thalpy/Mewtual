//! Warm original-aware completed-parent B verifier DATA. Exact comparisons bind all original
//! opaque SDK/Core outputs; this is not a cold-adoption or parsed-WAL permission.
use super::*;
pub(super) fn verify_completed_b_data(
    cell: &PreparedV2TextAcceptedCreate,
    key: &[u8; 32],
    wal: &TextWorkWalFrameData<'_>,
) -> Result<(), AppError> {
    let p = &cell.parent.head;
    if cell.completed.is_none() || !cell.signature.intent.retained.is_completed_parent_data() {
        return Err(refused());
    }
    cell.signature
        .intent
        .require_original_completed_carry(&RetirementCarryData::decode_data(
            &cell.signature.intent.carry,
        )?)?;
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
    if actual_candidate.append_data().old_tip().count != 1
        || actual_candidate.append_data().old_tip().count
            != cell.parent.directory.accepted_leaf_count
        || actual_candidate.append_data().new_tip().count != 2
        || p.queue_next != 1
        || p.queue_front > p.queue_next
        || actual_candidate.header_data() != &cell.parent.header
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
        || candidate.queue_front != p.queue_front
        || candidate.queue_next != p.queue_next.checked_add(1).ok_or_else(refused)?
        || candidate.live_bytes != p.live_bytes.checked_add(bytes).ok_or_else(refused)?
        || candidate.live_objects != p.live_objects.checked_add(count).ok_or_else(refused)?
        || bytes > REMAINING_AFTER_S_BYTES
        || count > REMAINING_AFTER_S_OBJECTS
    {
        return Err(refused());
    }
    if candidate.directory
        != PreparedV2TextAcceptedCreate::completed_accepted_root_data(
            key,
            cell.directory.as_ref().ok_or_else(refused)?,
            1,
        )?
        || candidate.tokens
            != PreparedV2TextAcceptedCreate::completed_private_root_data(
                key,
                cell.token.as_ref().ok_or_else(refused)?,
                PrivateIndexKindData::Token,
                p.context.id_data()?,
                2,
            )?
        || candidate.publications
            != PreparedV2TextAcceptedCreate::completed_private_root_data(
                key,
                cell.publications.as_ref().ok_or_else(refused)?,
                PrivateIndexKindData::Publication,
                p.context.id_data()?,
                2,
            )?
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
        || old.entries_data()
            != cell
                .signature
                .intent
                .retained
                .completed_retirement_entries_data()?
        || plan.carry_data().roots_data() != roots
    {
        return Err(refused());
    }
    let entries = plan.carry_data().entries_data();
    // Expected entries originate in the actual owning nextA's observed token path
    // and this completed query's authenticated Actions/publication ciphertexts.
    let expected_entries = cell.completed_retirement_entries_data()?;
    if entries != expected_entries {
        return Err(refused());
    }
    Ok(())
}

#[cfg(all(test, windows))]
mod tests;
